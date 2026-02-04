//! Context building for AI assistants in CodeGraph
//!
//! This crate builds relevant context from the knowledge graph for AI consumption.
//! It formats code relationships, source snippets, and documentation into
//! markdown or JSON suitable for LLM prompts.

pub mod builder;
pub mod error;
pub mod formatter;

pub use builder::{ContextBuilder, ContextOptions, ContextResult};
pub use error::ContextError;
pub use formatter::{ContextFormat, ContextFormatter};
