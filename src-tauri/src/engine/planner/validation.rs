use std::collections::{HashMap, HashSet};

use crate::engine::block::Block;
use crate::engine::budget::BudgetStatus;

use super::types::{ContextMutation, PendingPlan, PlanActions};
use super::ContextPlanner;

impl ContextPlanner {
    /// Strip a leading `#` from a block ID.
    ///
    /// LLMs often copy the display-formatted `#abc123` from tool output
    /// instead of the bare `abc123`. This normalizes both forms.
    fn normalize_block_id(id: &str) -> &str {
        id.strip_prefix('#').unwrap_or(id)
    }

    /// Validate raw plan actions against current engine state.
    /// Returns a `PendingPlan` with validated mutations and projected impact.
    pub fn validate_plan(
        &self,
        actions: &PlanActions,
        blocks: &[Block],
        budget: &BudgetStatus,
    ) -> Result<PendingPlan, Vec<String>> {
        let mut mutations = Vec::new();
        let mut errors = Vec::new();

        let block_ids: std::collections::HashSet<&str> =
            blocks.iter().map(|b| b.id.as_str()).collect();

        // Validate expand actions
        for id in &actions.expand {
            let nid = Self::normalize_block_id(id);
            if block_ids.contains(nid) {
                mutations.push(ContextMutation::Expand {
                    block_id: nid.to_string(),
                });
            } else {
                errors.push(format!("Block {nid} not found for expand"));
            }
        }

        // Validate archive actions
        for id in &actions.archive {
            let nid = Self::normalize_block_id(id);
            if !block_ids.contains(nid) {
                errors.push(format!("Block {nid} not found for archive"));
                continue;
            }
            // Reject archival of thinking blocks — Anthropic requires byte-identical preservation
            if let Some(block) = blocks.iter().find(|b| b.id == nid) {
                if block.role == crate::engine::types::Role::Thinking {
                    errors.push(format!(
                        "Block {nid} is a thinking block and cannot be archived (Anthropic requires byte-identical preservation)"
                    ));
                    continue;
                }
            }
            mutations.push(ContextMutation::Archive {
                block_id: nid.to_string(),
            });
        }

        // Validate recall actions (these reference archived blocks, not active ones)
        for id in &actions.recall {
            let nid = Self::normalize_block_id(id);
            mutations.push(ContextMutation::Recall {
                block_id: nid.to_string(),
            });
        }

        // Validate pin actions
        for id in &actions.pin {
            let nid = Self::normalize_block_id(id);
            if block_ids.contains(nid) {
                mutations.push(ContextMutation::Pin {
                    block_id: nid.to_string(),
                });
            } else {
                errors.push(format!("Block {nid} not found for pin"));
            }
        }

        // Validate unpin actions
        for id in &actions.unpin {
            let nid = Self::normalize_block_id(id);
            if block_ids.contains(nid) {
                mutations.push(ContextMutation::Unpin {
                    block_id: nid.to_string(),
                });
            } else {
                errors.push(format!("Block {nid} not found for unpin"));
            }
        }

        // Validate shift_to actions
        for (id, zone_str) in &actions.shift_to {
            let nid = Self::normalize_block_id(id);
            if !block_ids.contains(nid) {
                errors.push(format!("Block {nid} not found for shift"));
                continue;
            }
            match super::parse_builtin_zone(zone_str) {
                Some(zone) => {
                    mutations.push(ContextMutation::Shift {
                        block_id: nid.to_string(),
                        target_zone: zone,
                    });
                }
                None => {
                    errors.push(format!("Invalid zone '{zone_str}' for shift"));
                }
            }
        }

        // Validate compress actions
        for (id, summary) in &actions.compress {
            let nid = Self::normalize_block_id(id);
            if block_ids.contains(nid) {
                mutations.push(ContextMutation::Compress {
                    block_id: nid.to_string(),
                    summary: summary.clone(),
                });
            } else {
                errors.push(format!("Block {nid} not found for compress"));
            }
        }

        // Validate split actions
        for (thread_id, instruction) in &actions.split {
            mutations.push(ContextMutation::Split {
                thread_id: thread_id.clone(),
                at_turn: instruction.at,
                archive_before: instruction.archive_before,
            });
        }

        if !errors.is_empty() {
            return Err(errors);
        }

        // Compute projected impact
        let token_delta = self.estimate_token_delta(&mutations, blocks);
        let projected_block_count = Self::projected_block_count_from_mutations(blocks, &mutations);
        let projected_tokens = (budget.used_tokens as i64 + token_delta).max(0) as u64;
        let projected_utilization = if budget.limit_tokens > 0 {
            projected_tokens as f64 / budget.limit_tokens as f64
        } else {
            0.0
        };

        Ok(PendingPlan {
            mutations,
            token_delta,
            projected_block_count,
            projected_utilization,
        })
    }

    fn mutation_slot_key(mutation: &ContextMutation) -> String {
        match mutation {
            ContextMutation::Expand { block_id }
            | ContextMutation::Archive { block_id }
            | ContextMutation::Recall { block_id } => format!("presence:{block_id}"),
            ContextMutation::Pin { block_id } | ContextMutation::Unpin { block_id } => {
                format!("pin:{block_id}")
            }
            ContextMutation::Shift { block_id, .. } => format!("shift:{block_id}"),
            ContextMutation::Compress { block_id, .. }
            | ContextMutation::UpdateContent { block_id, .. } => format!("content:{block_id}"),
            ContextMutation::Split { thread_id, .. } => format!("split:{thread_id}"),
        }
    }

    pub(crate) fn merge_mutations(
        base: &[ContextMutation],
        append: &[ContextMutation],
    ) -> Vec<ContextMutation> {
        let mut merged: Vec<ContextMutation> = Vec::new();
        let mut slot_to_index: HashMap<String, usize> = HashMap::new();

        for mutation in base.iter().chain(append.iter()) {
            let slot = Self::mutation_slot_key(mutation);
            if let Some(existing_idx) = slot_to_index.get(&slot).copied() {
                merged[existing_idx] = mutation.clone();
            } else {
                slot_to_index.insert(slot, merged.len());
                merged.push(mutation.clone());
            }
        }

        merged
    }

    pub(crate) fn project_pending_plan_from_mutations(
        &self,
        mutations: Vec<ContextMutation>,
        blocks: &[Block],
        budget: &BudgetStatus,
    ) -> PendingPlan {
        let token_delta = self.estimate_token_delta(&mutations, blocks);
        let projected_block_count = Self::projected_block_count_from_mutations(blocks, &mutations);
        let projected_tokens = (budget.used_tokens as i64 + token_delta).max(0) as u64;
        let projected_utilization = if budget.limit_tokens > 0 {
            projected_tokens as f64 / budget.limit_tokens as f64
        } else {
            0.0
        };

        PendingPlan {
            mutations,
            token_delta,
            projected_block_count,
            projected_utilization,
        }
    }

    pub(crate) fn projected_block_count_from_mutations(
        blocks: &[Block],
        mutations: &[ContextMutation],
    ) -> usize {
        let archived_ids: HashSet<&str> = mutations
            .iter()
            .filter_map(|m| match m {
                ContextMutation::Archive { block_id } => Some(block_id.as_str()),
                _ => None,
            })
            .collect();
        let recalled_ids: HashSet<&str> = mutations
            .iter()
            .filter_map(|m| match m {
                ContextMutation::Recall { block_id } => Some(block_id.as_str()),
                _ => None,
            })
            .collect();

        blocks
            .len()
            .saturating_sub(archived_ids.len())
            .saturating_add(recalled_ids.len())
    }

    /// Approximate token count for an archival stub (e.g. `[archived: 5.0k tokens]`).
    const STUB_OVERHEAD_TOKENS: i64 = 10;

    /// Estimate net token delta from a set of mutations against current blocks.
    ///
    /// For archives, this mirrors the applicator's turn-grouping logic:
    /// - Full-turn archives (ALL blocks at a turn archived): full token savings.
    /// - Partial-turn archives: savings = block tokens minus stub overhead per block.
    pub(crate) fn estimate_token_delta(
        &self,
        mutations: &[ContextMutation],
        blocks: &[Block],
    ) -> i64 {
        let mut delta: i64 = 0;

        // Collect archived block IDs for turn-aware projection.
        let archived_ids: HashSet<&str> = mutations
            .iter()
            .filter_map(|m| match m {
                ContextMutation::Archive { block_id } => Some(block_id.as_str()),
                _ => None,
            })
            .collect();

        // Turn-aware archival projection: group blocks by turn_index and check coverage.
        if !archived_ids.is_empty() {
            let block_by_id: HashMap<&str, &Block> =
                blocks.iter().map(|b| (b.id.as_str(), b)).collect();

            // Group all blocks by turn_index.
            let mut blocks_per_turn: HashMap<u32, Vec<&Block>> = HashMap::new();
            for block in blocks {
                blocks_per_turn
                    .entry(block.metadata.turn_index)
                    .or_default()
                    .push(block);
            }

            // For each turn with archived blocks, compute delta.
            for blocks_at_turn in blocks_per_turn.values() {
                let archived_at_turn: Vec<&&Block> = blocks_at_turn
                    .iter()
                    .filter(|b| archived_ids.contains(b.id.as_str()))
                    .collect();

                if archived_at_turn.is_empty() {
                    continue;
                }

                if archived_at_turn.len() == blocks_at_turn.len() {
                    // Full-turn archive: entire message removed — full token savings.
                    for block in &archived_at_turn {
                        delta -= block.tokens as i64;
                    }
                } else {
                    // Partial-turn archive: content replaced with stubs.
                    for block in &archived_at_turn {
                        delta -= block.tokens as i64;
                        delta += Self::STUB_OVERHEAD_TOKENS;
                    }
                }
            }

            // Handle archived IDs that don't appear in blocks (defensive).
            for id in &archived_ids {
                if !block_by_id.contains_key(id) {
                    // Block not found — can't estimate.
                }
            }
        }

        // Non-archive mutations.
        for mutation in mutations {
            match mutation {
                ContextMutation::Archive { .. } => {
                    // Handled above in turn-aware logic.
                }
                ContextMutation::Compress {
                    block_id, summary, ..
                } => {
                    if let Some(block) = blocks.iter().find(|b| b.id == *block_id) {
                        // Rough estimate: summary tokens ~= summary.len() / 4
                        let summary_tokens = (summary.len() as i64) / 4;
                        delta -= block.tokens as i64;
                        delta += summary_tokens;
                    }
                }
                // Recall adds tokens (we don't know the exact amount without the archived block)
                ContextMutation::Recall { .. } => {
                    // Can't estimate without archived block data
                }
                // Expand, Pin, Unpin, Shift, Split, UpdateContent don't change total tokens
                _ => {}
            }
        }

        delta
    }
}
