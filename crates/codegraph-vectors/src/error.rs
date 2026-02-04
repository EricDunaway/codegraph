//! Vector error types

use codegraph_db::DbError;
use thiserror::Error;

/// Errors that can occur during vector operations
#[derive(Debug, Error)]
pub enum VectorError {
    /// Database error (from DbError)
    #[error("Database error: {0}")]
    Database(#[from] DbError),

    /// Database error (from rusqlite directly)
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    /// Model not found
    #[error("Model not found at {path}")]
    ModelNotFound { path: String },

    /// Model loading failed
    #[error("Failed to load model: {0}")]
    ModelLoadFailed(String),

    /// Tokenization failed
    #[error("Tokenization failed: {0}")]
    TokenizationFailed(String),

    /// Inference failed
    #[error("Inference failed: {0}")]
    InferenceFailed(String),

    /// Model integrity check failed
    #[error("Model integrity check failed: expected {expected}, got {actual}")]
    IntegrityCheckFailed { expected: String, actual: String },

    /// Vector dimension mismatch
    #[error("Vector dimension mismatch: expected {expected}, got {actual}")]
    DimensionMismatch { expected: usize, actual: usize },

    /// Feature not enabled
    #[error("Feature '{feature}' not enabled. Compile with --features {feature}")]
    FeatureNotEnabled { feature: String },

    /// IO error
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}
