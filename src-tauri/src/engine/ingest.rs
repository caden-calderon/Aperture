use std::collections::hash_map::DefaultHasher;
use std::collections::HashSet;
use std::hash::{Hash, Hasher};

use tracing::debug;

/// Write diagnostic line directly to /tmp/aperture-ingest.log (bypasses tracing buffering).
fn diag(msg: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/aperture-ingest.log")
    {
        let elapsed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let _ = writeln!(f, "[{elapsed}] {msg}");
    }
}

use super::*;

impl ContextEngine {
    /// Ingest blocks from a proxy capture.
    ///
    /// This is the main entry point called by the proxy handler after
    /// parsing a request/response exchange.
    #[allow(clippy::too_many_arguments)]
    pub fn ingest(
        &self,
        provider: &str,
        model: &str,
        source: &str,
        thread_id: Option<&str>,
        mut request_blocks: Vec<Block>,
        mut response_blocks: Vec<Block>,
        overhead_tokens: u32,
    ) -> IngestResult {
        // Ensure/get session
        let session_id = self.ensure_session(provider, model, source, thread_id);

        diag(&format!(
            "INGEST session={} thread={:?} req={} resp={}",
            &session_id[..8], thread_id, request_blocks.len(), response_blocks.len()
        ));

        // Filter out Claude Code internal prompts before processing
        request_blocks.retain(|b| !is_internal_prompt(b));

        // Filter out aperture context tool blocks — these are MCP/interceptor
        // tool_use/tool_result pairs that should not accumulate in the store.
        let is_context_tool =
            |b: &Block| b.metadata.tool_name.as_deref().is_some_and(crate::metacog::is_context_tool_name);
        request_blocks.retain(|b| !is_context_tool(b));
        response_blocks.retain(|b| !is_context_tool(b));

        // Strip ANSI escape codes before token counting
        for block in request_blocks.iter_mut().chain(response_blocks.iter_mut()) {
            let stripped = crate::util::strip_ansi_codes(&block.content);
            if stripped.len() != block.content.len() {
                block.compressed_versions.original.content = stripped.clone();
                block.content = stripped;
            }
        }

        // Response parsers currently emit turn_index=0 for assistant/tool output.
        // Normalize response turns so latest response content is classified and
        // ordered after the latest request turn in this exchange.
        if !request_blocks.is_empty() && !response_blocks.is_empty() {
            let next_turn = request_blocks
                .iter()
                .map(|b| b.metadata.turn_index)
                .max()
                .unwrap_or(0)
                .saturating_add(1);

            for block in &mut response_blocks {
                block.metadata.turn_index = next_turn;
                block.last_referenced_turn = next_turn;
            }
        }

        // Recount tokens accurately
        for block in request_blocks.iter_mut().chain(response_blocks.iter_mut()) {
            block.tokens = count_tokens(&block.content, model);
        }

        diag(&format!(
            "  after filters: req={} resp={}", request_blocks.len(), response_blocks.len()
        ));

        // Combine all blocks for this exchange
        let mut all_blocks: Vec<Block> = Vec::new();
        all_blocks.append(&mut request_blocks);
        all_blocks.append(&mut response_blocks);

        let exchange_tokens: u32 = all_blocks.iter().map(|b| b.tokens).sum();

        // Remove old session blocks — each API request contains the full
        // conversation history with new UUIDs, so we replace rather than accumulate.
        let old_block_ids = self
            .sessions
            .get(&session_id)
            .map(|s| s.block_ids.clone())
            .unwrap_or_default();
        let old_blocks = self.store.get_many(&old_block_ids);

        // Stabilize block IDs: match new blocks to old blocks by content so that
        // archival-induced OccurrenceTracker shifts don't change IDs for blocks
        // whose content hasn't changed. This prevents visual flicker in the UI.
        stabilize_block_ids(&mut all_blocks, &old_blocks);

        let block_ids: Vec<String> = all_blocks.iter().map(|b| b.id.clone()).collect();

        let is_subset = is_regressive_subset_capture(&old_block_ids, &block_ids);
        let is_collapse = is_regressive_semantic_collapse(&old_blocks, &all_blocks);

        diag(&format!(
            "  old={} new={} (after filter+combine) subset={} collapse={}",
            old_block_ids.len(), block_ids.len(), is_subset, is_collapse
        ));

        if is_subset || is_collapse {
            diag("  SKIPPED (regression guard)");
            let budget = self.session_budget_status(&session_id);
            let block_count = self
                .sessions
                .get(&session_id)
                .map(|s| s.block_ids.len())
                .unwrap_or(0);
            return IngestResult {
                session_id,
                block_count,
                total_tokens: budget.used_tokens,
                alert_level: budget.alert_level,
                applied: false,
            };
        }
        diag(&format!("  APPLIED → {} blocks", block_ids.len()));
        for old_id in &old_block_ids {
            self.dependencies.remove_block(old_id);
            self.versions.remove(old_id);
        }
        self.store.remove_many(&old_block_ids);

        // Store new blocks
        self.store.insert_many(all_blocks);

        // Replace session tracking (not accumulate)
        self.sessions.update(&session_id, |s| {
            s.block_ids = block_ids.clone();
            s.total_tokens = exchange_tokens;
            s.overhead_tokens = overhead_tokens;
            s.exchange_count += 1;
        });

        // Run classification pipeline on all session blocks
        let session_block_ids = self
            .sessions
            .get(&session_id)
            .map(|s| s.block_ids.clone())
            .unwrap_or_default();
        let mut session_blocks = self.store.get_many(&session_block_ids);

        let classification = classify(&mut session_blocks, &self.pipeline_config);

        // Apply zone changes back to store
        for block in &session_blocks {
            self.store.update(&block.id, |b| {
                b.zone = block.zone.clone();
                b.usage_heat = block.usage_heat;
            });
        }

        // Build dependency graph
        let edges = build_dependencies(&session_blocks);
        for edge in edges {
            self.dependencies.add_edge(edge);
        }

        // Update reference counts
        for block_id in &block_ids {
            let deps = self.dependencies.dependents_of(block_id);
            let ref_count = deps.len() as u32;
            self.store.update(block_id, |b| {
                b.reference_count = ref_count;
            });
        }

        // Log action
        let record = action_log::new_record(
            ActionActor::Pipeline,
            ActionKind::Ingest,
            block_ids.clone(),
            format!(
                "Ingested exchange from {provider}/{model} [{source}/{}]",
                thread_id.unwrap_or("default")
            ),
        );
        self.action_log.record(record);

        // Persist to SQLite (background, best-effort)
        self.persist_session(&session_id);

        // Emit context updated event — session-specific counts, not global store.
        let session_block_count = block_ids.len() as u32;
        let session_tokens = exchange_tokens.saturating_add(overhead_tokens);
        self.emit_context_updated(session_block_count, session_tokens);

        debug!(
            "Ingested exchange: session={}, source={}, thread={}, blocks={}, tokens={}, alert={:?}",
            session_id,
            source,
            thread_id.unwrap_or("default"),
            session_blocks.len(),
            session_tokens,
            classification.alert_level
        );

        IngestResult {
            session_id,
            block_count: session_blocks.len(),
            total_tokens: session_tokens,
            alert_level: classification.alert_level,
            applied: true,
        }
    }
}

/// Check if a block is a Claude Code internal prompt that should be filtered.
pub(crate) fn is_internal_prompt(block: &Block) -> bool {
    block.role == types::Role::User
        && (block.content.starts_with("[SUGGESTION MODE:") || block.content.starts_with("[S MODE:"))
}

pub(crate) fn is_regressive_subset_capture(
    old_block_ids: &[String],
    new_block_ids: &[String],
) -> bool {
    if old_block_ids.is_empty() || new_block_ids.is_empty() {
        return false;
    }

    let old: HashSet<&str> = old_block_ids.iter().map(|id| id.as_str()).collect();
    let new: HashSet<&str> = new_block_ids.iter().map(|id| id.as_str()).collect();

    let removed = old.difference(&new).count();
    let added = new.difference(&old).count();

    removed > 0 && added == 0
}

pub(crate) fn is_regressive_semantic_collapse(old_blocks: &[Block], new_blocks: &[Block]) -> bool {
    if old_blocks.is_empty() || new_blocks.is_empty() {
        return false;
    }

    // Extreme collapse: 5+ blocks down to 1-2 is always regressive.
    // This catches MCP tool-call bursts (e.g., /context) where the context-tool
    // filter strips most blocks, leaving only 1-2 that have novel content. Without
    // this, the fingerprint check below sees "new content" and lets it through.
    if old_blocks.len() > 4 && new_blocks.len() <= 2 {
        return true;
    }

    // Guard only severe drops; normal archival commits should still apply.
    if new_blocks.len().saturating_mul(2) > old_blocks.len() {
        return false;
    }

    let old_fingerprints: HashSet<u64> =
        old_blocks.iter().map(block_semantic_fingerprint).collect();
    let new_fingerprints: HashSet<u64> =
        new_blocks.iter().map(block_semantic_fingerprint).collect();

    let removed = old_fingerprints.difference(&new_fingerprints).count();
    if removed == 0 {
        return false;
    }

    let added_blocks: Vec<&Block> = new_blocks
        .iter()
        .filter(|block| !old_fingerprints.contains(&block_semantic_fingerprint(block)))
        .collect();

    if added_blocks.is_empty() {
        return true;
    }

    added_blocks
        .iter()
        .all(|block| is_ephemeral_regression_addition(block))
}

fn block_semantic_fingerprint(block: &Block) -> u64 {
    let normalized = normalize_regression_content(block);
    let mut hasher = DefaultHasher::new();
    block.role.hash(&mut hasher);
    block.metadata.tool_name.hash(&mut hasher);
    normalized.hash(&mut hasher);
    hasher.finish()
}

fn normalize_regression_content(block: &Block) -> String {
    let raw = if block.role == Role::System {
        block
            .content
            .lines()
            .filter(|line| {
                let trimmed = line.trim_start().to_ascii_lowercase();
                !trimmed.starts_with("x-anthropic-billing-header:")
            })
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        block.content.clone()
    };
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_ephemeral_regression_addition(block: &Block) -> bool {
    match block.role {
        Role::ToolUse | Role::ToolResult | Role::Thinking => true,
        Role::User => is_transient_user_wrapper(&block.content),
        _ => false,
    }
}

fn is_transient_user_wrapper(content: &str) -> bool {
    let trimmed = content.trim_start();
    trimmed.starts_with("<system-reminder>")
        || trimmed.starts_with("<local-command-caveat>")
        || trimmed.starts_with("<local-command-stdout>")
        || trimmed.starts_with("<command-name>")
        || trimmed.starts_with("<command-message>")
        || trimmed.starts_with("<command-args>")
}

/// Match new blocks to existing session blocks by content to preserve IDs.
///
/// The parser's `OccurrenceTracker` resets per parse, so when archival removes
/// a block that shares a (role, content_fingerprint) with another block, the
/// remaining block's occurrence index shifts and it gets a new ID. This causes
/// Svelte's keyed `{#each}` to unmount/remount with transitions.
///
/// This function greedily matches new blocks to old blocks by (role, content_prefix)
/// and reuses the old block's ID, keeping the UI stable.
fn stabilize_block_ids(new_blocks: &mut [Block], old_blocks: &[Block]) {
    use std::collections::HashMap;

    // Build pool of reusable IDs indexed by (role, content_prefix_200).
    let mut pool: HashMap<(String, String), Vec<String>> = HashMap::new();
    for old in old_blocks {
        let key = block_content_key(old);
        pool.entry(key).or_default().push(old.id.clone());
    }

    // Greedily match new blocks to old IDs.
    for block in new_blocks.iter_mut() {
        let key = block_content_key(block);
        if let Some(ids) = pool.get_mut(&key) {
            if let Some(reused) = ids.pop() {
                block.id = reused;
            }
        }
    }
}

fn block_content_key(block: &Block) -> (String, String) {
    let prefix: String = block.content.chars().take(200).collect();
    (format!("{:?}", block.role), prefix)
}
