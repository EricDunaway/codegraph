//! SQLite database layer for CodeGraph
//!
//! This crate provides the database abstraction for storing and querying
//! the code knowledge graph.

#![allow(dead_code, unused_variables)] // TODO: Remove when implemented

pub mod connection;
pub mod enrichment_deps;
pub mod error;
pub mod migrations;
pub mod queries;
pub mod schema;

pub use connection::DatabaseConnection;
pub use enrichment_deps::{
    clear_deps_for_file, clear_deps_for_node, clear_deps_for_nodes, count_deps_for_node,
    get_deps_for_node, get_nodes_depending_on_file, get_nodes_depending_on_files,
    insert_enrichment_dep, insert_enrichment_deps_batch,
};
pub use error::DbError;
pub use migrations::{get_schema_version, migrate_to_v2, run_migrations};
pub use queries::QueryBuilder;

// Re-export rusqlite for external access to Connection type
pub use rusqlite;
