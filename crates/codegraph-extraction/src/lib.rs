//! Tree-sitter based code extraction for CodeGraph
//!
//! This crate handles parsing source files and extracting nodes and edges
//! to build the code knowledge graph.
//!
//! ## Architecture
//!
//! The extraction system uses a configuration-driven approach:
//!
//! - `TreeSitterParser` - Wraps tree-sitter for multi-language parsing
//! - `TreeSitterExtractor` - Generic extractor that works with any configured language
//! - `LanguageConfig` - Defines how to map tree-sitter AST nodes to CodeGraph nodes
//! - `ExtractorRegistry` - Thread-safe interface for extraction
//!
//! This design avoids language-specific extractor implementations. Instead,
//! language differences are captured in configuration (node type mappings,
//! decorator detection, etc.).

pub mod error;
pub mod scanner;
pub mod extractor;
pub mod orchestrator;
pub mod parser;
pub mod languages;
pub mod tree_sitter_extractor;

pub use error::ExtractionError;
pub use scanner::{FileScanner, ScanResult};
pub use extractor::{ExtractorRegistry, generate_node_id};
pub use orchestrator::{ExtractionOrchestrator, IndexProgress, IndexResult, SyncResult};
pub use parser::TreeSitterParser;
pub use tree_sitter_extractor::TreeSitterExtractor;
pub use languages::{LanguageConfig, get_language_config};
