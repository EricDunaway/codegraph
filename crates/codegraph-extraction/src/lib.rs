//! Tree-sitter based code extraction for CodeGraph
//!
//! This crate handles parsing source files and extracting nodes and edges
//! to build the code knowledge graph.

pub mod error;
pub mod scanner;
pub mod extractor;
pub mod orchestrator;

pub use error::ExtractionError;
pub use scanner::{FileScanner, ScanResult};
pub use extractor::{LanguageExtractor, ExtractorRegistry};
pub use orchestrator::{ExtractionOrchestrator, IndexProgress, IndexResult, SyncResult};
