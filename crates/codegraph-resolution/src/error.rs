//! Resolution error types

use codegraph_db::DbError;
use thiserror::Error;

/// Errors that can occur during reference resolution
#[derive(Debug, Error)]
pub enum ResolutionError {
    /// Database error
    #[error("Database error: {0}")]
    Database(#[from] DbError),

    /// Node not found
    #[error("Node not found: {0}")]
    NodeNotFound(String),

    /// Ambiguous reference (multiple candidates)
    #[error("Ambiguous reference '{name}': {count} candidates found")]
    AmbiguousReference { name: String, count: usize },

    /// Resolution failed
    #[error("Failed to resolve reference: {0}")]
    ResolutionFailed(String),
}
