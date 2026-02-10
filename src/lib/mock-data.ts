/**
 * Mock Data Generator
 * Realistic demo context data for Phase 0 UI development
 */

import type {
  Block,
  Role,
  Zone,
  CompressionLevel,
  Session,
  Snapshot,
  TokenBudget,
} from "./types";

// ============================================================================
// ID Generation
// ============================================================================

let idCounter = 0;

function generateId(): string {
  return `block-${Date.now()}-${++idCounter}`;
}

// ============================================================================
// Token Estimation (rough approximation for mock data)
// ============================================================================

function estimateTokens(content: string): number {
  // Rough estimate: ~4 characters per token for English text
  return Math.ceil(content.length / 4);
}

// ============================================================================
// Block Factory
// ============================================================================

interface CreateBlockOptions {
  role: Role;
  content: string;
  zone?: Zone;
  usageHeat?: number;
  compressionLevel?: CompressionLevel;
  toolName?: string;
}

function createBlock(options: CreateBlockOptions): Block {
  const {
    role,
    content,
    zone = "middle",
    usageHeat = Math.random() * 0.5 + 0.2,
    compressionLevel = "original",
    toolName,
  } = options;

  const tokens = estimateTokens(content);

  return {
    id: generateId(),
    role,
    content,
    tokens,
    timestamp: new Date(Date.now() - Math.random() * 3600000), // Random time in last hour
    zone,
    pinned: null,
    compressionLevel,
    compressedVersions: {
      original: { content, tokens },
    },
    usageHeat,
    positionRelevance: zone === "primacy" ? 0.9 : zone === "recency" ? 0.7 : 0.4,
    lastReferencedTurn: Math.floor(Math.random() * 10),
    referenceCount: Math.floor(Math.random() * 5),
    topicCluster: null,
    topicKeywords: [],
    metadata: {
      provider: "anthropic",
      turnIndex: Math.floor(Math.random() * 20),
      toolName,
      filePaths: [],
    },
  };
}

// ============================================================================
// Sample Content
// ============================================================================

const SYSTEM_PROMPT = `You are Claude, an AI assistant by Anthropic. You are helping with the Aperture project, a universal LLM context visualization tool.

Key guidelines:
- Follow the planning-first workflow
- Use Svelte 5 runes ($state, $derived, $effect)
- Maintain the halftone/dithering aesthetic
- Every pixel encodes data — no decorative elements`;

const USER_MESSAGES = [
  "Let's build the Zone component. It should display blocks in a collapsible container with a header showing the zone name, token count, and block count.",
  "The blocks should be draggable. When I drag a block to a different zone, it should move there. Add visual feedback during drag.",
  "Can you add selection? Click to select one block, shift+click for range select, and show a teal border on selected blocks.",
  "Now implement the compression slider. When I drag the slider, blocks should visually dissolve based on their compression level.",
  "We need a token budget bar at the top. Show usage by zone with a gradient that changes color as we approach the limit.",
  "Add a command palette like VS Code. Cmd+K should open it with all available actions.",
  "The sidebar needs sections for snapshots, block types, and keyboard shortcuts. Make it collapsible.",
  "Implement multi-select drag. When I have multiple blocks selected, dragging one should move all of them.",
  "Add custom zones. Users should be able to create their own zones beyond primacy/middle/recency.",
  "The theme system needs work. Add preset themes and let users customize individual colors.",
];

const ASSISTANT_RESPONSES = [
  `I'll create the Zone component with a collapsible header. The design will follow the halftone aesthetic:

\`\`\`svelte
<script lang="ts">
  import type { Block, Zone } from '$lib/types';

  let { zone, blocks, collapsed = $bindable(false) }: Props = $props();

  let totalTokens = $derived(blocks.reduce((sum, b) => sum + b.tokens, 0));
</script>
\`\`\`

The header will show zone name with the accent color, plus token/block counts in JetBrains Mono.`,

  `For drag-and-drop, I'll use svelte-dnd-action. Key interactions:

1. **Drag start**: Block lifts with subtle shadow, origin zone dims slightly
2. **Drag over**: Target zone highlights with dithered border
3. **Drop**: Block animates to new position with spring physics

The zone accent colors (teal/yellow/pink) will pulse on valid drop targets.`,

  `Selection implementation with the Aperture aesthetic:

- **Click**: Instant 2px teal border (decisive, no fade)
- **Shift+click**: Range select with subtle cascade animation
- **Ctrl+click**: Toggle individual selection
- **A key**: Select all in current zone

Selected blocks get intensified halftone dots to show "heat" increase.`,

  `The compression slider will use a continuous range from 0-100%. Visual feedback:

- **0-30%**: Full opacity, all content visible
- **30-60%**: Slight blur, dots start to scatter
- **60-90%**: Heavy dithering, text barely legible
- **90-100%**: Nearly invisible, just scattered dots remain

This creates the "dissolving document" effect that defines Aperture's aesthetic.`,

  `TokenBudgetBar implementation with zone segments:

\`\`\`svelte
<div class="budget-bar">
  {#each zoneSegments as segment}
    <div class="segment" style:width="{segment.percent}%" style:background={segment.color}></div>
  {/each}
</div>
\`\`\`

Colors shift from warm gold (safe) to rust red (danger) as usage increases. At 90%+, the bar pulses.`,

  `CommandPalette with fuzzy search:

- Opens on Cmd+K (Mac) or Ctrl+K (Windows)
- Shows all available commands grouped by category
- Type to filter, Enter to execute
- Recent commands appear at top

Styled with the newspaper aesthetic - no rounded corners, strong borders.`,

  `Sidebar architecture with collapsible sections:

1. **Snapshots**: List of saved states with restore buttons
2. **Block Types**: Filter by system/user/assistant/tool
3. **Display**: Density slider (75%-125%)
4. **Theme**: Preset selector and customization
5. **Shortcuts**: Quick reference card

Each section persists its collapsed state to localStorage.`,

  `Multi-drag implementation:

When dragging with selection, I show a badge "+N" indicating how many blocks are moving. All selected blocks animate together with staggered delays for a cascading effect.

The drop zone shows "Drop 4 blocks to Recency" for clarity.`,

  `Custom zones stored in zonesStore:

\`\`\`typescript
interface ZoneConfig {
  id: string;
  label: string;
  color: string;
  isBuiltIn: boolean;
  contextOrder: number;  // Position in LLM context
  displayOrder: number;  // Visual position in UI
}
\`\`\`

Primacy always first, Recency always last in context order.`,

  `Theme system with 13 built-in presets:

**Dark themes**: Charcoal, Tokyo Night, Gruvbox, Catppuccin, Nord, Dracula, One Dark, Solarized
**Light themes**: Warm, Gruvbox Light, Tokyo Light, Sepia, Solarized Light

Users can customize any of 18 color properties. Custom themes persist to localStorage.`,
];

const TOOL_RESULTS = [
  {
    name: "Read",
    content: `export type Role = "system" | "user" | "assistant" | "tool_use" | "tool_result";
export type Zone = "primacy" | "middle" | "recency";
export type CompressionLevel = "original" | "trimmed" | "summarized" | "minimal";

export interface Block {
  id: string;
  role: Role;
  content: string;
  tokens: number;
  timestamp: Date;
  zone: Zone;
  pinned: "top" | "bottom" | null;
  compressionLevel: CompressionLevel;
  usageHeat: number;
  positionRelevance: number;
  metadata: BlockMetadata;
}`,
  },
  {
    name: "Read",
    content: `use axum::{Router, routing::post, extract::State, response::IntoResponse};
use tokio::sync::RwLock;
use std::sync::Arc;

#[derive(Clone)]
struct AppState {
    sessions: Arc<RwLock<HashMap<String, Session>>>,
    proxy_config: ProxyConfig,
}

async fn forward_request(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<impl IntoResponse, ProxyError> {
    let upstream = detect_provider(&headers)?;
    let response = state.proxy_config
        .client
        .post(&upstream.url)
        .headers(redact_sensitive(&headers))
        .body(body)
        .send()
        .await?;
    Ok(response)
}

pub fn create_router(state: AppState) -> Router {
    Router::new()
        .route("/v1/messages", post(forward_request))
        .route("/v1/chat/completions", post(forward_request))
        .with_state(state)
}`,
  },
  {
    name: "Edit",
    content: `Updated src/lib/components/Zone.svelte
- Added collapse toggle
- Added token count display
- Styled with zone accent color`,
  },
  {
    name: "Read",
    content: `import numpy as np
from dataclasses import dataclass
from typing import Optional

@dataclass
class TokenStats:
    total: int
    by_role: dict[str, int]
    by_zone: dict[str, int]
    compression_ratio: float

def calculate_compression_savings(
    blocks: list[dict],
    target_level: str = "trimmed",
) -> TokenStats:
    """Calculate token savings for a given compression level."""
    original_tokens = sum(b["tokens"] for b in blocks)
    compressed = [compress_block(b, target_level) for b in blocks]
    new_tokens = sum(b["tokens"] for b in compressed)

    return TokenStats(
        total=new_tokens,
        by_role=_group_by(compressed, "role"),
        by_zone=_group_by(compressed, "zone"),
        compression_ratio=new_tokens / original_tokens if original_tokens > 0 else 1.0,
    )`,
  },
  {
    name: "Bash",
    content: `$ cargo test --manifest-path src-tauri/Cargo.toml
   Compiling aperture v0.1.0
    Finished test [unoptimized + debuginfo] target(s) in 4.23s
     Running unittests src/lib.rs
running 12 tests
test proxy::tests::test_detect_anthropic ... ok
test proxy::tests::test_detect_openai ... ok
test proxy::tests::test_sse_streaming ... ok
test engine::tests::test_block_creation ... ok
test engine::tests::test_zone_assignment ... ok
test engine::tests::test_token_counting ... ok
test engine::tests::test_compression_original ... ok
test engine::tests::test_compression_trimmed ... ok
test engine::tests::test_compression_summarized ... ok
test terminal::tests::test_session_spawn ... ok
test terminal::tests::test_session_resize ... ok
test terminal::tests::test_session_cleanup ... ok
test result: ok. 12 passed; 0 failed; 0 ignored`,
  },
  {
    name: "Read",
    content: `{
  "name": "aperture",
  "version": "0.1.0",
  "private": true,
  "scripts": {
    "dev": "vite dev",
    "build": "vite build",
    "preview": "vite preview",
    "check": "svelte-check --tsconfig ./tsconfig.json",
    "tauri": "tauri"
  },
  "dependencies": {
    "@tauri-apps/api": "^2.0.0",
    "@tauri-apps/plugin-shell": "^2.0.0",
    "@xterm/xterm": "^5.5.0",
    "@xterm/addon-fit": "^0.10.0",
    "prismjs": "^1.30.0"
  },
  "devDependencies": {
    "@sveltejs/adapter-static": "^3.0.0",
    "@sveltejs/kit": "^2.0.0",
    "@sveltejs/vite-plugin-svelte": "^4.0.0",
    "svelte": "^5.0.0",
    "tailwindcss": "^4.0.0",
    "typescript": "^5.0.0",
    "vite": "^6.0.0"
  }
}`,
  },
  {
    name: "Read",
    content: `apiVersion: apps/v1
kind: Deployment
metadata:
  name: aperture-proxy
  namespace: ai-tools
  labels:
    app: aperture
    component: proxy
spec:
  replicas: 2
  selector:
    matchLabels:
      app: aperture
  template:
    spec:
      containers:
        - name: proxy
          image: aperture/proxy:latest
          ports:
            - containerPort: 5400
          env:
            - name: ANTHROPIC_API_KEY
              valueFrom:
                secretKeyRef:
                  name: api-keys
                  key: anthropic
          resources:
            requests:
              memory: "128Mi"
              cpu: "250m"
            limits:
              memory: "512Mi"
              cpu: "1000m"`,
  },
  {
    name: "Read",
    content: `.zone-header {
  display: flex;
  align-items: center;
  padding: 6px 10px;
  gap: 8px;
  cursor: pointer;
  user-select: none;
  transition: background 0.1s ease;
  border-radius: 4px 4px 0 0;
}

.zone-header:hover {
  background: var(--bg-hover);
}

.zone-accent {
  width: 8px;
  height: 8px;
  border-radius: 2px;
  flex-shrink: 0;
}

.zone-name {
  font-family: var(--font-display);
  font-size: 11px;
  font-weight: 600;
  color: var(--text-primary);
  letter-spacing: 0.02em;
}

.zone-stats {
  font-family: var(--font-mono);
  font-size: 10px;
  color: var(--text-muted);
  margin-left: auto;
}`,
  },
  {
    name: "Edit",
    content: `Updated src/lib/stores/theme.svelte.ts
- Added 13 built-in presets
- Implemented color customization
- localStorage persistence`,
  },
];

// ============================================================================
// Demo Session Generator
// ============================================================================

export function generateDemoBlocks(): Block[] {
  const blocks: Block[] = [];

  // System prompt in primacy zone (high heat, pinned)
  const systemBlock = createBlock({
    role: "system",
    content: SYSTEM_PROMPT,
    zone: "primacy",
    usageHeat: 0.95,
  });
  systemBlock.pinned = "top";
  blocks.push(systemBlock);

  // Additional primacy blocks (project context, always included)
  blocks.push(
    createBlock({
      role: "system",
      content: `Project: Aperture - Universal LLM Context Proxy\nStack: Tauri v2 + Svelte 5 + Rust (axum)\nPhase: 0 - UI Foundation\nAesthetic: Halftone/dithering, Obra Dinn meets IDE`,
      zone: "primacy",
      usageHeat: 0.9,
    })
  );
  blocks.push(
    createBlock({
      role: "system",
      content: `Code Standards:\n- Svelte 5 runes: $state, $derived, $effect\n- Rust: thiserror for domain errors, anyhow for app errors\n- Tailwind for layout, custom CSS for effects\n- JetBrains Mono for code, IBM Plex Mono for headers`,
      zone: "primacy",
      usageHeat: 0.85,
    })
  );
  blocks.push(
    createBlock({
      role: "system",
      content: `Architecture decisions:\n- Proxy via ANTHROPIC_BASE_URL / OPENAI_API_BASE env vars\n- Three layers: Rust proxy core, Rust context engine, Svelte 5 + Canvas UI\n- Non-destructive compression: originals always preserved\n- Cleaner model sidecar for background tasks`,
      zone: "primacy",
      usageHeat: 0.88,
    })
  );

  // Conversation turns in middle zone
  for (let i = 0; i < USER_MESSAGES.length; i++) {
    // User message
    blocks.push(
      createBlock({
        role: "user",
        content: USER_MESSAGES[i],
        zone: "middle",
        usageHeat: 0.3 + Math.random() * 0.3,
      })
    );

    // Tool use/result occasionally
    if (i < TOOL_RESULTS.length && Math.random() > 0.3) {
      blocks.push(
        createBlock({
          role: "tool_use",
          content: `Calling ${TOOL_RESULTS[i].name} tool...`,
          zone: "middle",
          usageHeat: 0.2 + Math.random() * 0.2,
          toolName: TOOL_RESULTS[i].name,
        })
      );
      blocks.push(
        createBlock({
          role: "tool_result",
          content: TOOL_RESULTS[i].content,
          zone: "middle",
          usageHeat: 0.4 + Math.random() * 0.3,
          toolName: TOOL_RESULTS[i].name,
        })
      );
    }

    // Assistant response
    if (i < ASSISTANT_RESPONSES.length) {
      blocks.push(
        createBlock({
          role: "assistant",
          content: ASSISTANT_RESPONSES[i],
          zone: "middle",
          usageHeat: 0.5 + Math.random() * 0.3,
        })
      );
    }
  }

  // Recent context in recency zone (more blocks to demonstrate scrolling)
  blocks.push(
    createBlock({
      role: "user",
      content:
        "Great progress! Let's add keyboard shortcuts next. A for select all, Esc to deselect, Del to remove.",
      zone: "recency",
      usageHeat: 0.8,
    })
  );

  blocks.push(
    createBlock({
      role: "assistant",
      content: `I'll implement the keyboard shortcuts using Svelte's on:keydown handler at the window level:

\`\`\`svelte
<svelte:window on:keydown={handleKeydown} />
\`\`\`

The shortcuts will:
- **A**: Select all visible blocks
- **Esc**: Clear selection
- **Del**: Remove selected (with dissolution animation)
- **C**: Open compression slider for selected
- **S**: Quick save snapshot`,
      zone: "recency",
      usageHeat: 0.85,
    })
  );

  blocks.push(
    createBlock({
      role: "user",
      content: "Can you also add Ctrl+Z for undo? And make the snapshot restore reversible.",
      zone: "recency",
      usageHeat: 0.82,
    })
  );

  blocks.push(
    createBlock({
      role: "assistant",
      content: `For undo/redo, I'll implement a command history stack:\n\n1. Each action (move, delete, compress) pushes to the stack\n2. Ctrl+Z pops and reverses the last action\n3. Ctrl+Shift+Z redoes\n4. Stack is limited to 50 entries to prevent memory issues\n\nSnapshot restore will auto-create a "before restore" snapshot first.`,
      zone: "recency",
      usageHeat: 0.87,
    })
  );

  blocks.push(
    createBlock({
      role: "user",
      content: "Perfect. One more thing — add a visual indicator showing which blocks changed since the last snapshot.",
      zone: "recency",
      usageHeat: 0.9,
    })
  );

  return blocks;
}

export function generateDemoSession(): Session {
  const blocks = generateDemoBlocks();

  return {
    id: "demo-session",
    name: "Aperture UI Development",
    provider: "anthropic",
    tokenLimit: 200000,
    blocks,
    createdAt: new Date(Date.now() - 3600000),
    updatedAt: new Date(),
  };
}

export function generateDemoSnapshots(_currentBlocks: Block[]): Snapshot[] {
  // No demo snapshots — working state is the default.
  // Users can save snapshots manually via the sidebar.
  return [];
}

export function calculateTokenBudget(blocks: Block[], limit = 200_000): TokenBudget {
  const budget: TokenBudget = {
    used: 0,
    limit,
    byZone: { primacy: 0, middle: 0, recency: 0 }, // Initialize built-in zones
    byRole: {
      system: 0,
      user: 0,
      assistant: 0,
      tool_use: 0,
      tool_result: 0,
      thinking: 0,
    },
  };

  for (const block of blocks) {
    budget.used += block.tokens;
    // Initialize zone if not exists (for custom zones)
    if (!(block.zone in budget.byZone)) {
      budget.byZone[block.zone] = 0;
    }
    budget.byZone[block.zone] += block.tokens;
    budget.byRole[block.role] += block.tokens;
  }

  return budget;
}
