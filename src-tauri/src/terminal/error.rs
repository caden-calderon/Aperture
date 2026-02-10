use thiserror::Error;

#[derive(Debug, Error)]
pub enum TerminalError {
    #[error("Failed to spawn shell: {0}")]
    SpawnFailed(String),

    #[error("Session not found: {0}")]
    SessionNotFound(String),

    #[error("Failed to write to terminal: {0}")]
    WriteFailed(String),

    #[error("Failed to resize terminal: {0}")]
    ResizeFailed(String),

    #[error("Failed to initialize terminal reader: {0}")]
    ReaderInitFailed(String),
}

impl serde::Serialize for TerminalError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}
