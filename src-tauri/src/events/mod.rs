//! Event system for Aperture.
//!
//! Defines the events that flow between the proxy, engine, and frontend.
//! Events are emitted via Tauri's event system and consumed by the
//! Svelte frontend for real-time updates.

pub mod broadcaster;
pub mod dispatcher;
pub mod types;
