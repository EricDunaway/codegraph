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
pub mod snippet;
pub mod errors;
pub mod test_detection;
pub mod package;
pub mod tree_sitter_extractor;

pub use error::ExtractionError;
pub use scanner::{FileScanner, ScanResult};
pub use extractor::{ExtractorRegistry, generate_node_id};
pub use orchestrator::{ExtractionOrchestrator, IndexProgress, IndexResult, SyncResult};
pub use parser::TreeSitterParser;
pub use tree_sitter_extractor::TreeSitterExtractor;
pub use languages::{LanguageConfig, get_language_config};
pub use snippet::{extract_code_snippet, extract_code_snippet_for_range, DEFAULT_MAX_LINES, TRUNCATION_MARKER};
pub use errors::extract_thrown_errors;
pub use test_detection::{
    is_test_file, find_test_names_for_symbol, extract_imports, associate_tests_via_imports,
    find_tests_for_symbol, ImportInfo, AssociationMethod, TestAssociation,
};
pub use package::{
    extract_package_name, extract_package_name_nearest, extract_package_for_file,
    is_workspace_root, PackageInfo, PackageSource,
};
