//! LSP error types

use thiserror::Error;

/// Errors that can occur during LSP operations
#[derive(Debug, Error)]
pub enum LspError {
    /// Server failed to start
    #[error("Failed to start LSP server: {0}")]
    StartFailed(String),

    /// Server not ready (workspace not initialized)
    #[error("LSP server not ready: {0}")]
    NotReady(String),

    /// Request timed out
    #[error("LSP request timed out: {0}")]
    Timeout(String),

    /// Server returned an error
    #[error("LSP server error: {code} - {message}")]
    ServerError { code: i32, message: String },

    /// Invalid response from server
    #[error("Invalid LSP response: {0}")]
    InvalidResponse(String),

    /// IO error
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON serialization error
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// Server shutdown unexpectedly
    #[error("LSP server shutdown unexpectedly")]
    ServerShutdown,

    /// Language not supported
    #[error("Language not supported for LSP: {0}")]
    UnsupportedLanguage(String),

    /// File not found
    #[error("File not found: {0}")]
    FileNotFound(String),

    /// Position out of bounds
    #[error("Position out of bounds: line {line}, column {column}")]
    PositionOutOfBounds { line: u32, column: u32 },
}
