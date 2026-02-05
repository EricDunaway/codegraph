//! CodeGraph Core Library
//!
//! This is the main library crate that exposes the public API for CodeGraph,
//! a local-first code intelligence system that builds a semantic knowledge graph
//! from any codebase.
//!
//! # Example
//!
//! ```no_run
//! use codegraph_core::CodeGraph;
//!
//! let mut cg = CodeGraph::init("/path/to/project").unwrap();
//! cg.index_all().unwrap();
//!
//! // Search for symbols
//! let results = cg.search("myFunction", 10).unwrap();
//!
//! // Get context for a task
//! let context = cg.build_context("implement feature X").unwrap();
//! ```

mod codegraph;
mod config;
pub mod embedding;
mod error;

pub use codegraph::CodeGraph;
pub use config::{CodeGraphConfig, DEFAULT_EXCLUDE_PATTERNS};
pub use embedding::{build_embedding_text, build_embedding_text_with_budget};
pub use error::CodeGraphError;

// Re-export types
pub use codegraph_types::*;

// Re-export sub-crates for advanced usage
pub use codegraph_context as context;
pub use codegraph_db as db;
pub use codegraph_extraction as extraction;
pub use codegraph_graph as graph;
pub use codegraph_resolution as resolution;
pub use codegraph_sync as sync;
pub use codegraph_vectors as vectors;
