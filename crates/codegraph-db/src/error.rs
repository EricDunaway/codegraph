//! Database error types

use thiserror::Error;

/// Database error type
#[derive(Debug, Error)]
pub enum DbError {
    /// SQLite error
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    /// Schema error
    #[error("Schema error: {message}")]
    Schema { message: String },

    /// Query error
    #[error("Query error during {operation}: {message}")]
    Query { operation: String, message: String },

    /// Migration error
    #[error("Migration error: {message}")]
    Migration { message: String },

    /// Not found error
    #[error("{entity} not found: {id}")]
    NotFound { entity: String, id: String },

    /// Constraint violation
    #[error("Constraint violation: {message}")]
    Constraint { message: String },
}
