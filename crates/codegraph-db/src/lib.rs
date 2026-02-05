//! SQLite database layer for CodeGraph
//!
//! This crate provides the database abstraction for storing and querying
//! the code knowledge graph.

#![allow(dead_code, unused_variables)] // TODO: Remove when implemented

pub mod connection;
pub mod error;
pub mod migrations;
pub mod queries;
pub mod schema;

pub use connection::DatabaseConnection;
pub use error::DbError;
pub use migrations::{get_schema_version, migrate_to_v2, run_migrations};
pub use queries::QueryBuilder;

// Re-export rusqlite for external access to Connection type
pub use rusqlite;
