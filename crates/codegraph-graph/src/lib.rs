//! Graph traversal algorithms for CodeGraph
//!
//! This crate provides BFS, DFS, call graph construction, impact analysis,
//! and other graph algorithms for the code knowledge graph.

pub mod error;
pub mod traversal;
pub mod queries;

pub use error::GraphError;
pub use traversal::{GraphTraverser, TraversalResult};
pub use queries::{
    CallGraph, CircularDependency, DeadCodeResult, GraphQueryManager, ImpactRadius, NodeMetrics,
};
