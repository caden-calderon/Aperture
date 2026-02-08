/**
 * Aperture Type Definitions
 * Universal block format and core types
 */

// ============================================================================
// Block Types
// ============================================================================

export type Role = "system" | "user" | "assistant" | "tool_use" | "tool_result" | "thinking";

// Built-in zones + custom zones (custom zones are string IDs like "zone-123")
export type Zone = "primacy" | "middle" | "recency" | (string & {});

export type PinPosition = "top" | "bottom";

export type CompressionLevel = "original" | "trimmed" | "summarized" | "minimal";

export interface CompressionVersion {
  content: string;
  tokens: number;
}

export interface CompressionVersions {
  original: CompressionVersion;
  trimmed?: CompressionVersion;
  summarized?: CompressionVersion;
  minimal?: CompressionVersion;
}

export interface BlockMetadata {
  provider: string;
  turnIndex: number;
  toolName?: string;
  filePaths: string[];
}

export interface Block {
  id: string;
  role: Role; // Canonical API role
  blockType?: string; // Optional UI taxonomy id; display identity is blockType ?? role
  content: string;
  tokens: number;
  timestamp: Date;
  zone: Zone;
  pinned: PinPosition | null;

  // Compression (non-destructive, multi-level)
  compressionLevel: CompressionLevel;
  compressedVersions: CompressionVersions;

  // Heat & Attention
  usageHeat: number; // 0.0-1.0
  positionRelevance: number; // 0.0-1.0
  lastReferencedTurn: number;
  referenceCount: number;

  // Topic Clustering
  topicCluster: string | null;
  topicKeywords: string[];

  // Metadata
  metadata: BlockMetadata;
}

// ============================================================================
// Session Types
// ============================================================================

export interface Session {
  id: string;
  name: string;
  provider: string;
  tokenLimit: number;
  blocks: Block[];
  createdAt: Date;
  updatedAt: Date;
}

// ============================================================================
// Snapshot Types
// ============================================================================

export interface SnapshotZoneState {
  customZones: Array<{id: string; label: string; color: string; isBuiltIn: boolean; contextOrder: number; displayOrder: number}>;
  displayOrderOverrides: Record<string, number>;
  builtInOverrides: Record<string, { label?: string; color?: string }>;
  zoneHeights: Record<string, number>;
  expandedZones: string[];
  contentExpandedZones: string[];
  tokenHistory: Record<string, number[]>;
}

export interface Snapshot {
  id: string;
  name: string;
  timestamp: Date;
  blocks: Block[];
  totalTokens: number;
  type: "hard" | "soft";
  zoneState?: SnapshotZoneState | null;
  parentSnapshotId?: string | null;
}

// ============================================================================
// UI State Types
// ============================================================================

export interface SelectionState {
  selectedIds: Set<string>;
  lastSelectedIndex: number | null;
}

export interface UIState {
  expandedBlocks: Set<string>;
  modalBlockId: string | null;
  commandPaletteOpen: boolean;
  draggingBlockId: string | null;
}

// ============================================================================
// Token Budget Types
// ============================================================================

export interface TokenBudget {
  used: number;
  limit: number;
  byZone: Record<string, number>; // Dynamic zones
  byRole: Record<Role, number>;
}

// ============================================================================
// Rule Types
// ============================================================================

export interface Rule {
  id: string;
  name: string;
  trigger: RuleTrigger;
  action: RuleAction;
  config: Record<string, unknown>;
  enabled: boolean;
}

export interface RuleTrigger {
  field: string;
  operator: "eq" | "gt" | "lt" | "contains" | "matches";
  value: string | number;
}

export type RuleAction =
  | "condense"
  | "remove"
  | "pin"
  | "unpin"
  | "move_zone"
  | "warn";

// ============================================================================
// Event Types (for IPC)
// ============================================================================

export type ContextEvent =
  | { type: "blocks_updated"; blocks: Block[] }
  | { type: "selection_changed"; selectedIds: string[] }
  | { type: "token_count_changed"; budget: TokenBudget }
  | { type: "snapshot_created"; snapshot: Snapshot }
  | { type: "snapshot_restored"; snapshotId: string };

// ============================================================================
// Utility Types
// ============================================================================

export type BlockFilter = {
  roles?: Role[];
  zones?: Zone[];
  minTokens?: number;
  maxTokens?: number;
  minHeat?: number;
  maxHeat?: number;
  clusters?: string[];
};

export function isRole(value: string): value is Role {
  return ["system", "user", "assistant", "tool_use", "tool_result", "thinking"].includes(
    value
  );
}

export function isZone(value: string): value is Zone {
  return ["primacy", "middle", "recency"].includes(value);
}

export function isCompressionLevel(value: string): value is CompressionLevel {
  return ["original", "trimmed", "summarized", "minimal"].includes(value);
}
