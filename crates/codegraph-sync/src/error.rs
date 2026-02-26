//! Sync error types

use codegraph_db::DbError;
use codegraph_extraction::ExtractionError;
use thiserror::Error;

/// Errors that can occur during sync operations
#[derive(Debug, Error)]
pub enum SyncError {
    /// Database error
    #[error("Database error: {0}")]
    Database(#[from] DbError),

    /// Extraction error
    #[error("Extraction error: {0}")]
    Extraction(#[from] ExtractionError),

    /// IO error
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Git error
    #[error("Git error: {message}")]
    Git { message: String },

    /// Hook installation failed
    #[error("Failed to install hook '{hook}': {message}")]
    HookInstallFailed { hook: String, message: String },

    /// File not found
    #[error("File not found: {0}")]
    FileNotFound(String),

    /// Invalid configuration
    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),

    /// Lock acquisition failed
    #[error("Failed to acquire lock: {0}")]
    LockFailed(String),

    /// Lock is already held by another process
    #[error("Lock already held")]
    LockHeld,

    /// Hook manager detected (Husky, Lefthook, etc.)
    #[error("Hook manager detected: {tool}. Use --force to override.")]
    HookManagerDetected { tool: String },

    /// Other error
    #[error("{0}")]
    Other(String),
}
