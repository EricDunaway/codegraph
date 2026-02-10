//! Vector embeddings for semantic search in CodeGraph
//!
//! This crate handles embedding generation and similarity search for code symbols.
//! Uses direct `ort` for ONNX inference with optional CoreML acceleration on macOS.
//!
//! # Features
//!
//! - `onnx` - Enable ONNX runtime for actual embedding generation
//! - `coreml` - Enable CoreML acceleration on macOS (implies `onnx`)
//!
//! Without features, provides mock implementations for testing.

pub mod embedder;
pub mod error;
pub mod search;
pub mod storage;
pub mod text_builder;

pub use embedder::{EmbedderConfig, TextEmbedder};
pub use error::VectorError;
pub use search::{cosine_similarity, SearchConfig, SimilarityResult, SimilaritySearch};
pub use storage::VectorStorage;
pub use text_builder::{EmbeddingTextBuilder, GraphContext, NodeEnrichment, TokenCounter};
