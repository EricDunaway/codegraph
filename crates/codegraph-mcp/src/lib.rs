//! Model Context Protocol server for CodeGraph
//!
//! This crate provides an MCP server for IDE integration.
//! It exposes CodeGraph functionality through JSON-RPC over stdio.

pub mod error;
pub mod git;
pub mod protocol;
pub mod server;
pub mod tools;

pub use error::McpError;
pub use server::McpServer;
pub use tools::McpTools;
