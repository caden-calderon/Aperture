/**
 * Block conversion utilities — Rust snake_case → TypeScript camelCase.
 *
 * Shared between connection store (event reception) and context store
 * (engine IPC fetches).
 */

import type { Block } from "../types";

// ============================================================================
// Types
// ============================================================================

/** Rust Block struct — snake_case fields from serde. */
export interface RustBlock {
  id: string;
  role: string;
  block_type: string | null;
  content: string;
  tokens: number;
  timestamp: string;
  zone: string | { BuiltIn: string } | { Custom: string };
  pinned: string | null;
  compression_level: string;
  compressed_versions: {
    original: { content: string; tokens: number };
    trimmed?: { content: string; tokens: number };
    summarized?: { content: string; tokens: number };
    minimal?: { content: string; tokens: number };
  };
  usage_heat: number;
  position_relevance: number;
  last_referenced_turn: number;
  reference_count: number;
  topic_cluster: string | null;
  topic_keywords: string[];
  metadata: {
    provider: string;
    turn_index: number;
    tool_name: string | null;
    file_paths: string[];
  };
}

/** Payload from the Rust ApertureEvent::BlocksCaptured variant. */
export interface BlocksCapturedPayload {
  type: "blocks_captured";
  request_id: string;
  provider: string;
  model: string;
  request_blocks: RustBlock[];
  response_blocks: RustBlock[];
  input_tokens: number | null;
  output_tokens: number | null;
}

// ============================================================================
// Conversion
// ============================================================================

export function convertZone(zone: RustBlock["zone"]): string {
  if (typeof zone === "string") return zone;
  if ("BuiltIn" in zone) return (zone.BuiltIn as string).toLowerCase();
  if ("Custom" in zone) return zone.Custom;
  return "recency";
}

export function convertBlock(rb: RustBlock): Block {
  return {
    id: rb.id,
    role: rb.role as Block["role"],
    blockType: rb.block_type ?? undefined,
    content: rb.content,
    tokens: rb.tokens,
    timestamp: new Date(rb.timestamp),
    zone: convertZone(rb.zone),
    pinned: (rb.pinned as Block["pinned"]) ?? null,
    compressionLevel: rb.compression_level as Block["compressionLevel"],
    compressedVersions: {
      original: rb.compressed_versions.original,
      trimmed: rb.compressed_versions.trimmed,
      summarized: rb.compressed_versions.summarized,
      minimal: rb.compressed_versions.minimal,
    },
    usageHeat: rb.usage_heat,
    positionRelevance: rb.position_relevance,
    lastReferencedTurn: rb.last_referenced_turn,
    referenceCount: rb.reference_count,
    topicCluster: rb.topic_cluster,
    topicKeywords: rb.topic_keywords,
    metadata: {
      provider: rb.metadata.provider,
      turnIndex: rb.metadata.turn_index,
      toolName: rb.metadata.tool_name ?? undefined,
      filePaths: rb.metadata.file_paths,
    },
  };
}
