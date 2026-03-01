//! Graph error types

use thiserror::Error;

/// Errors that can occur during graph operations
#[derive(Debug, Error)]
pub enum GraphError {
    /// Node not found
    #[error("Node not found: {0}")]
    NodeNotFound(String),

    /// Cycle detected
    #[error("Cycle detected involving node: {0}")]
    CycleDetected(String),

    /// Database error
    #[error("Database error: {0}")]
    Database(#[from] codegraph_db::DbError),

    /// Invalid traversal options
    #[error("Invalid traversal options: {0}")]
    InvalidOptions(String),

    /// Traversal limit exceeded
    #[error("Traversal limit exceeded: visited {0} nodes")]
    LimitExceeded(usize),
}
