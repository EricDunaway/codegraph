//! CodeGraph error types

use codegraph_context::ContextError;
use codegraph_db::DbError;
use codegraph_extraction::ExtractionError;
use codegraph_graph::GraphError;
use codegraph_resolution::ResolutionError;
use codegraph_sync::SyncError;
use codegraph_vectors::VectorError;
use std::path::PathBuf;
use thiserror::Error;

/// Errors that can occur in CodeGraph operations
#[derive(Debug, Error)]
pub enum CodeGraphError {
    /// Database error
    #[error("Database error: {0}")]
    Database(#[from] DbError),

    /// Extraction error
    #[error("Extraction error: {0}")]
    Extraction(#[from] ExtractionError),

    /// Graph error
    #[error("Graph error: {0}")]
    Graph(#[from] GraphError),

    /// Resolution error
    #[error("Resolution error: {0}")]
    Resolution(#[from] ResolutionError),

    /// Context error
    #[error("Context error: {0}")]
    Context(#[from] ContextError),

    /// Vector error
    #[error("Vector error: {0}")]
    Vector(#[from] VectorError),

    /// Sync error
    #[error("Sync error: {0}")]
    Sync(#[from] SyncError),

    /// IO error
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Project not initialized
    #[error("Project not initialized at {0}")]
    NotInitialized(PathBuf),

    /// Project already initialized
    #[error("Project already initialized at {0}")]
    AlreadyInitialized(PathBuf),

    /// Invalid path
    #[error("Invalid path: {0}")]
    InvalidPath(PathBuf),

    /// Node not found
    #[error("Node not found: {0}")]
    NodeNotFound(String),

    /// Configuration error
    #[error("Configuration error: {0}")]
    Config(String),

    /// Embedding error
    #[error("Embedding error: {0}")]
    Embedding(String),
}
