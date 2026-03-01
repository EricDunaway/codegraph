//! MCP error types

use codegraph_context::ContextError;
use codegraph_db::DbError;
use codegraph_graph::GraphError;
use thiserror::Error;

/// Errors that can occur in the MCP server
#[derive(Debug, Error)]
pub enum McpError {
    /// Database error
    #[error("Database error: {0}")]
    Database(#[from] DbError),

    /// Graph error
    #[error("Graph error: {0}")]
    Graph(#[from] GraphError),

    /// Context error
    #[error("Context error: {0}")]
    Context(#[from] ContextError),

    /// JSON serialization error
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// IO error
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Invalid request
    #[error("Invalid request: {0}")]
    InvalidRequest(String),

    /// Tool not found
    #[error("Tool not found: {0}")]
    ToolNotFound(String),

    /// Invalid parameters
    #[error("Invalid parameters: {0}")]
    InvalidParams(String),

    /// Node not found
    #[error("Node not found: {0}")]
    NodeNotFound(String),

    /// Server not initialized
    #[error("Server not initialized")]
    NotInitialized,
}
