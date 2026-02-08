//! Core type definitions for the context engine.

use serde::{Deserialize, Serialize};

/// Role of a context block in the conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    System,
    User,
    Assistant,
    ToolUse,
    ToolResult,
    Thinking,
}

/// Built-in context zones.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Zone {
    /// Built-in zone variant.
    #[serde(rename_all = "snake_case")]
    BuiltIn(BuiltInZone),
    /// Custom user-defined zone.
    Custom(String),
}

/// The three built-in zones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuiltInZone {
    Primacy,
    Middle,
    Recency,
}

/// Compression level for a context block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompressionLevel {
    Original,
    Trimmed,
    Summarized,
    Minimal,
}

/// Pin position within a zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PinPosition {
    Top,
    Bottom,
}
