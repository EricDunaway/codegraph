//! Context error types

use codegraph_db::DbError;
use codegraph_graph::GraphError;
use thiserror::Error;

/// Errors that can occur during context building
#[derive(Debug, Error)]
pub enum ContextError {
    /// Database error
    #[error("Database error: {0}")]
    Database(#[from] DbError),

    /// Graph error
    #[error("Graph error: {0}")]
    Graph(#[from] GraphError),

    /// Node not found
    #[error("Node not found: {0}")]
    NodeNotFound(String),

    /// File read error
    #[error("Failed to read file {path}: {message}")]
    FileRead { path: String, message: String },

    /// Serialization error
    #[error("Serialization error: {0}")]
    Serialization(String),

    /// Context too large
    #[error("Context exceeds maximum size ({actual} > {max})")]
    ContextTooLarge { actual: usize, max: usize },
}
