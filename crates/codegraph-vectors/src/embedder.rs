//! Text embedding generation
//!
//! Provides embedding generation using ONNX models.
//! When the `onnx` feature is not enabled, provides a mock implementation.

use crate::error::VectorError;
use sha2::{Digest, Sha256};
use std::path::PathBuf;

#[cfg(feature = "onnx")]
use std::path::Path;

/// Default model filename
pub const DEFAULT_MODEL_FILENAME: &str = "nomic-embed-text-v1.5.onnx";
/// Default tokenizer filename
pub const DEFAULT_TOKENIZER_FILENAME: &str = "tokenizer.json";
/// Models subdirectory name
pub const MODELS_DIR: &str = "models";
/// CodeGraph directory name
pub const CODEGRAPH_DIR: &str = ".codegraph";

/// Configuration for the embedder
#[derive(Debug, Clone)]
pub struct EmbedderConfig {
    /// Path to the ONNX model file (if None, will search standard locations)
    pub model_path: Option<PathBuf>,
    /// Path to the tokenizer file (if None, will search standard locations)
    pub tokenizer_path: Option<PathBuf>,
    /// Expected SHA256 hash of the model (for integrity verification)
    pub model_hash: Option<String>,
    /// Maximum sequence length
    pub max_length: usize,
    /// Output dimension
    pub dimension: usize,
}

impl Default for EmbedderConfig {
    fn default() -> Self {
        Self {
            model_path: None,
            tokenizer_path: None,
            model_hash: None,
            max_length: 512,
            dimension: 768, // nomic-embed default
        }
    }
}

impl EmbedderConfig {
    /// Create config with explicit model paths
    pub fn with_paths(model_path: impl Into<PathBuf>, tokenizer_path: impl Into<PathBuf>) -> Self {
        Self {
            model_path: Some(model_path.into()),
            tokenizer_path: Some(tokenizer_path.into()),
            ..Default::default()
        }
    }
}

/// Text embedder
pub struct TextEmbedder {
    config: EmbedderConfig,
    #[cfg(feature = "onnx")]
    session: Option<ort::session::Session>,
    #[cfg(feature = "onnx")]
    tokenizer: Option<tokenizers::Tokenizer>,
}

impl TextEmbedder {
    /// Create a new embedder (without loading the model)
    pub fn new(config: EmbedderConfig) -> Self {
        Self {
            config,
            #[cfg(feature = "onnx")]
            session: None,
            #[cfg(feature = "onnx")]
            tokenizer: None,
        }
    }

    /// Check if the model is loaded
    pub fn is_loaded(&self) -> bool {
        #[cfg(feature = "onnx")]
        {
            self.session.is_some() && self.tokenizer.is_some()
        }
        #[cfg(not(feature = "onnx"))]
        {
            false
        }
    }

    /// Get the output dimension
    pub fn dimension(&self) -> usize {
        self.config.dimension
    }

    /// Load the model
    pub fn load(&mut self) -> Result<(), VectorError> {
        #[cfg(feature = "onnx")]
        {
            self.load_onnx()
        }
        #[cfg(not(feature = "onnx"))]
        {
            Err(VectorError::FeatureNotEnabled {
                feature: "onnx".to_string(),
            })
        }
    }

    /// Get the standard model search paths (in priority order)
    ///
    /// Search order:
    /// 1. Project-level: `.codegraph/models/`
    /// 2. User-level: `~/.codegraph/models/`
    pub fn model_search_paths() -> Vec<PathBuf> {
        let mut paths = Vec::new();

        // 1. Project-level: .codegraph/models/
        let project_path = PathBuf::from(CODEGRAPH_DIR).join(MODELS_DIR);
        paths.push(project_path);

        // 2. User-level: ~/.codegraph/models/
        if let Some(home) = dirs_home() {
            let user_path = home.join(CODEGRAPH_DIR).join(MODELS_DIR);
            paths.push(user_path);
        }

        paths
    }

    /// Resolve model file path by searching standard locations
    ///
    /// If `explicit_path` is Some, uses that directly.
    /// Otherwise searches project-level then user-level directories.
    pub fn resolve_model_path(
        explicit_path: Option<&PathBuf>,
        filename: &str,
    ) -> Result<PathBuf, VectorError> {
        // If explicit path provided, use it directly
        if let Some(path) = explicit_path {
            if path.exists() {
                return Ok(path.clone());
            }
            return Err(VectorError::ModelNotFound {
                path: path.display().to_string(),
            });
        }

        // Search standard locations
        for search_dir in Self::model_search_paths() {
            let candidate = search_dir.join(filename);
            if candidate.exists() {
                log::info!("Found model at: {}", candidate.display());
                return Ok(candidate);
            }
            log::debug!("Model not found at: {}", candidate.display());
        }

        // Not found - provide helpful error message
        let search_paths: Vec<String> = Self::model_search_paths()
            .iter()
            .map(|p| p.join(filename).display().to_string())
            .collect();

        Err(VectorError::ModelNotFound {
            path: format!(
                "{} (searched: {})",
                filename,
                search_paths.join(", ")
            ),
        })
    }

    /// Load ONNX model (only available with onnx feature)
    #[cfg(feature = "onnx")]
    fn load_onnx(&mut self) -> Result<(), VectorError> {
        // Resolve model path
        let model_path = Self::resolve_model_path(
            self.config.model_path.as_ref(),
            DEFAULT_MODEL_FILENAME,
        )?;

        // Resolve tokenizer path
        let tokenizer_path = Self::resolve_model_path(
            self.config.tokenizer_path.as_ref(),
            DEFAULT_TOKENIZER_FILENAME,
        )?;

        // Verify integrity if hash provided
        if let Some(ref expected_hash) = self.config.model_hash {
            let actual_hash = self.compute_file_hash(&model_path)?;
            if actual_hash != *expected_hash {
                return Err(VectorError::IntegrityCheckFailed {
                    expected: expected_hash.clone(),
                    actual: actual_hash,
                });
            }
        }

        // Load tokenizer
        let tokenizer = tokenizers::Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| VectorError::TokenizationFailed(e.to_string()))?;

        // Build session with execution providers
        // CoreML is auto-enabled on macOS Apple Silicon (M1/M2/M3/M4) via target-specific dependencies
        let session = self.build_session(&model_path)?;

        self.session = Some(session);
        self.tokenizer = Some(tokenizer);

        log::info!("Loaded embedding model from {}", model_path.display());
        log::info!("Loaded tokenizer from {}", tokenizer_path.display());
        Ok(())
    }

    /// Build ONNX session with appropriate execution providers
    /// CoreML is automatically used on macOS Apple Silicon for Neural Engine + GPU acceleration
    #[cfg(feature = "onnx")]
    fn build_session(&self, model_path: &Path) -> Result<ort::session::Session, VectorError> {
        use ort::session::Session;

        // On macOS Apple Silicon, try CoreML execution provider for Neural Engine + GPU acceleration
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        {
            use ort::ep::CoreML;

            // Check if CODEGRAPH_NO_COREML env var is set to skip CoreML
            if std::env::var("CODEGRAPH_NO_COREML").is_ok() {
                log::info!("CoreML disabled via CODEGRAPH_NO_COREML, using CPU");
                return Session::builder()
                    .map_err(|e| VectorError::ModelLoadFailed(e.to_string()))?
                    .commit_from_file(model_path)
                    .map_err(|e| VectorError::ModelLoadFailed(e.to_string()));
            }

            let coreml_ep = CoreML::default()
                .with_subgraphs(true) // Enable for all subgraphs
                .build();

            match Session::builder()
                .map_err(|e| VectorError::ModelLoadFailed(e.to_string()))?
                .with_execution_providers([coreml_ep])
            {
                Ok(builder) => {
                    log::info!("Using CoreML execution provider (Apple Silicon)");
                    builder
                        .commit_from_file(model_path)
                        .map_err(|e| VectorError::ModelLoadFailed(e.to_string()))
                }
                Err(e) => {
                    log::warn!(
                        "CoreML execution provider failed to register, falling back to CPU: {}",
                        e
                    );
                    // Fall back to CPU-only session
                    Session::builder()
                        .map_err(|e| VectorError::ModelLoadFailed(e.to_string()))?
                        .commit_from_file(model_path)
                        .map_err(|e| VectorError::ModelLoadFailed(e.to_string()))
                }
            }
        }

        // On all other platforms, use default CPU execution
        #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
        {
            Session::builder()
                .map_err(|e| VectorError::ModelLoadFailed(e.to_string()))?
                .commit_from_file(model_path)
                .map_err(|e| VectorError::ModelLoadFailed(e.to_string()))
        }
    }

    /// Compute SHA256 hash of a file
    #[cfg(feature = "onnx")]
    fn compute_file_hash(&self, path: &Path) -> Result<String, VectorError> {
        let content = std::fs::read(path)?;
        let mut hasher = Sha256::new();
        hasher.update(&content);
        let hash = hasher.finalize();
        Ok(format!("{:x}", hash))
    }

    /// Generate embedding for a single text (raw, no prefix)
    ///
    /// For models that use query/document prefixes (like nomic-embed-text-v1.5),
    /// prefer `embed_query()` or `embed_document()` instead.
    pub fn embed(&mut self, text: &str) -> Result<Vec<f32>, VectorError> {
        #[cfg(feature = "onnx")]
        {
            self.embed_onnx(text)
        }
        #[cfg(not(feature = "onnx"))]
        {
            // Mock implementation for testing without ONNX
            Ok(self.mock_embed(text))
        }
    }

    /// Generate embedding for a search query
    ///
    /// Prepends the `search_query: ` prefix required by nomic-embed-text-v1.5
    /// for asymmetric query-document retrieval.
    pub fn embed_query(&mut self, query: &str) -> Result<Vec<f32>, VectorError> {
        let prefixed = format!("search_query: {}", query);
        self.embed(&prefixed)
    }

    /// Generate embedding for a document (code symbol text)
    ///
    /// Prepends the `search_document: ` prefix required by nomic-embed-text-v1.5
    /// for asymmetric query-document retrieval.
    pub fn embed_document(&mut self, document: &str) -> Result<Vec<f32>, VectorError> {
        let prefixed = format!("search_document: {}", document);
        self.embed(&prefixed)
    }

    /// Generate embeddings for multiple texts (raw, no prefix)
    pub fn embed_batch(&mut self, texts: &[&str]) -> Result<Vec<Vec<f32>>, VectorError> {
        texts.iter().map(|t| self.embed(t)).collect()
    }

    /// ONNX embedding implementation
    #[cfg(feature = "onnx")]
    fn embed_onnx(&mut self, text: &str) -> Result<Vec<f32>, VectorError> {
        use ort::value::Tensor;

        // Get config values first to avoid borrow issues
        let max_length = self.config.max_length;
        let dimension = self.config.dimension;

        let tokenizer = self
            .tokenizer
            .as_ref()
            .ok_or_else(|| VectorError::TokenizationFailed("Tokenizer not loaded".to_string()))?;

        // Tokenize
        let encoding = tokenizer
            .encode(text, true)
            .map_err(|e| VectorError::TokenizationFailed(e.to_string()))?;

        let input_ids: Vec<i64> = encoding.get_ids().iter().map(|&id| id as i64).collect();
        let attention_mask: Vec<i64> = encoding
            .get_attention_mask()
            .iter()
            .map(|&m| m as i64)
            .collect();
        // token_type_ids are all zeros for single-sentence encoding (BERT-style models)
        let token_type_ids: Vec<i64> = encoding
            .get_type_ids()
            .iter()
            .map(|&t| t as i64)
            .collect();

        let seq_len = input_ids.len().min(max_length);

        // Create ort Tensors using shape + vec pattern (ort 2.0 API)
        let input_ids_tensor =
            Tensor::from_array(([1usize, seq_len], input_ids[..seq_len].to_vec()))
                .map_err(|e| VectorError::InferenceFailed(e.to_string()))?;
        let attention_mask_tensor =
            Tensor::from_array(([1usize, seq_len], attention_mask[..seq_len].to_vec()))
                .map_err(|e| VectorError::InferenceFailed(e.to_string()))?;
        let token_type_ids_tensor =
            Tensor::from_array(([1usize, seq_len], token_type_ids[..seq_len].to_vec()))
                .map_err(|e| VectorError::InferenceFailed(e.to_string()))?;

        let session = self
            .session
            .as_mut()
            .ok_or_else(|| VectorError::ModelLoadFailed("Model not loaded".to_string()))?;

        // Run inference using ort::inputs! macro
        let outputs = session
            .run(ort::inputs![
                "input_ids" => input_ids_tensor,
                "attention_mask" => attention_mask_tensor,
                "token_type_ids" => token_type_ids_tensor
            ])
            .map_err(|e| VectorError::InferenceFailed(e.to_string()))?;

        // Extract embedding (usually last_hidden_state or pooler_output)
        let output = outputs
            .get("last_hidden_state")
            .or_else(|| outputs.get("sentence_embedding"))
            .ok_or_else(|| VectorError::InferenceFailed("No output tensor found".to_string()))?;

        // Extract the tensor data - try_extract_tensor returns (&Shape, &[T])
        let (_, data) = output
            .try_extract_tensor::<f32>()
            .map_err(|e| VectorError::InferenceFailed(e.to_string()))?;

        // Determine output shape: token-level [1, seq_len, dim] vs sentence-level [1, dim]
        let total_elements = data.len();
        if total_elements > dimension && total_elements % dimension == 0 {
            // Token-level embeddings (e.g. last_hidden_state [1, seq_len, dim]):
            // apply mean pooling across tokens
            let output_seq_len = total_elements / dimension;
            Ok(mean_pool(data, output_seq_len, dimension))
        } else {
            // Sentence-level embedding (e.g. sentence_embedding [1, dim]):
            // use directly, truncating to configured dimension
            let embedding: Vec<f32> = data.iter().take(dimension).copied().collect();
            Ok(embedding)
        }
    }

    /// Mock embedding for testing (deterministic based on text hash)
    #[cfg(not(feature = "onnx"))]
    fn mock_embed(&self, text: &str) -> Vec<f32> {
        // Generate deterministic pseudo-random embedding from text hash
        let mut hasher = Sha256::new();
        hasher.update(text.as_bytes());
        let hash = hasher.finalize();

        let mut embedding = Vec::with_capacity(self.config.dimension);
        let mut idx = 0;

        while embedding.len() < self.config.dimension {
            // Use hash bytes to generate floats
            let byte1 = hash[idx % 32] as f32;
            let byte2 = hash[(idx + 1) % 32] as f32;
            let value = (byte1 * 256.0 + byte2) / 65535.0 * 2.0 - 1.0; // Normalize to [-1, 1]
            embedding.push(value);
            idx += 2;
        }

        // Normalize to unit length
        let norm: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
        for v in &mut embedding {
            *v /= norm;
        }

        embedding
    }
}

/// Get user home directory (cross-platform)
fn dirs_home() -> Option<PathBuf> {
    // Try HOME env var first (Unix), then USERPROFILE (Windows)
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// Mean pooling over token embeddings (standalone function to avoid borrow issues)
#[cfg(feature = "onnx")]
fn mean_pool(embeddings: &[f32], seq_len: usize, dim: usize) -> Vec<f32> {
    let mut result = vec![0.0f32; dim];

    for token_idx in 0..seq_len {
        for (i, v) in result.iter_mut().enumerate() {
            *v += embeddings[token_idx * dim + i];
        }
    }

    for v in &mut result {
        *v /= seq_len as f32;
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_embedder_config_default() {
        let config = EmbedderConfig::default();
        assert_eq!(config.dimension, 768);
        assert_eq!(config.max_length, 512);
        assert!(config.model_path.is_none());
        assert!(config.tokenizer_path.is_none());
    }

    #[test]
    fn test_embedder_config_with_paths() {
        let config = EmbedderConfig::with_paths("/path/to/model.onnx", "/path/to/tokenizer.json");
        assert_eq!(
            config.model_path,
            Some(PathBuf::from("/path/to/model.onnx"))
        );
        assert_eq!(
            config.tokenizer_path,
            Some(PathBuf::from("/path/to/tokenizer.json"))
        );
    }

    #[test]
    fn test_model_search_paths() {
        let paths = TextEmbedder::model_search_paths();

        // Debug: print paths
        for (i, path) in paths.iter().enumerate() {
            eprintln!("Search path {}: {}", i, path.display());
        }

        // Should have at least project-level path
        assert!(!paths.is_empty());
        // First path should be project-level
        assert!(paths[0].ends_with("models"));
        assert!(paths[0].to_string_lossy().contains(".codegraph"));

        // Should have user-level path if HOME is set
        if std::env::var("HOME").is_ok() {
            assert!(paths.len() >= 2, "Should have user-level path when HOME is set");
            let home = std::env::var("HOME").unwrap();
            let expected_user_path = format!("{}/.codegraph/models", home);
            assert_eq!(paths[1].to_string_lossy(), expected_user_path);
        }
    }

    #[test]
    fn test_embedder_not_loaded_by_default() {
        let config = EmbedderConfig::default();
        let embedder = TextEmbedder::new(config);
        assert!(!embedder.is_loaded());
    }

    #[test]
    #[cfg(not(feature = "onnx"))]
    fn test_mock_embedding() {
        let config = EmbedderConfig {
            dimension: 384,
            ..Default::default()
        };
        let mut embedder = TextEmbedder::new(config);

        let embedding = embedder.embed("Hello world").unwrap();
        assert_eq!(embedding.len(), 384);

        // Check normalization (should be unit vector)
        let norm: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 0.001);
    }

    #[test]
    #[cfg(not(feature = "onnx"))]
    fn test_mock_embedding_deterministic() {
        let config = EmbedderConfig {
            dimension: 384,
            ..Default::default()
        };
        let mut embedder = TextEmbedder::new(config);

        let e1 = embedder.embed("Test text").unwrap();
        let e2 = embedder.embed("Test text").unwrap();

        // Same text should produce same embedding
        for (a, b) in e1.iter().zip(e2.iter()) {
            assert!((a - b).abs() < 1e-9);
        }
    }

    #[test]
    #[cfg(not(feature = "onnx"))]
    fn test_mock_embedding_different_texts() {
        let config = EmbedderConfig {
            dimension: 384,
            ..Default::default()
        };
        let mut embedder = TextEmbedder::new(config);

        let e1 = embedder.embed("Hello").unwrap();
        let e2 = embedder.embed("World").unwrap();

        // Different texts should produce different embeddings
        let similarity: f32 = e1.iter().zip(e2.iter()).map(|(a, b)| a * b).sum();
        assert!(similarity < 0.99); // Not identical
    }

    #[test]
    #[cfg(not(feature = "onnx"))]
    fn test_embed_batch() {
        let config = EmbedderConfig {
            dimension: 384,
            ..Default::default()
        };
        let mut embedder = TextEmbedder::new(config);

        let texts = vec!["Hello", "World", "Test"];
        let embeddings = embedder.embed_batch(&texts).unwrap();

        assert_eq!(embeddings.len(), 3);
        for e in &embeddings {
            assert_eq!(e.len(), 384);
        }
    }

    #[test]
    #[cfg(not(feature = "onnx"))]
    fn test_integration_text_builder_with_embedder() {
        use crate::text_builder::{EmbeddingTextBuilder, GraphContext, NodeEnrichment};
        use codegraph_types::{EmbeddingTextConfig, Language, Node, NodeKind};

        // Build text using text_builder
        let text_config = EmbeddingTextConfig::default();
        let text_builder = EmbeddingTextBuilder::new(text_config);

        let mut node = Node::new(
            "test-fn",
            NodeKind::Function,
            "processPayment",
            "PaymentService.processPayment",
            "src/payment.ts",
            Language::TypeScript,
            10,
            25,
        );
        node.decorators = vec!["@Controller".to_string()];
        node.signature = Some("async processPayment(order: Order): Promise<Receipt>".to_string());

        let context = GraphContext {
            callees: vec!["validateOrder".to_string(), "chargeCard".to_string()],
            callers: vec!["handleCheckout".to_string()],
            ..Default::default()
        };

        let enrichment = NodeEnrichment {
            inferred_type: Some("Promise<Receipt>".to_string()),
            package_name: Some("@myapp/payments".to_string()),
            ..Default::default()
        };

        let text = text_builder.build_text(&node, &context, &enrichment);

        // Verify text has expected content
        assert!(text.contains("@Controller"));
        assert!(text.contains("processPayment"));
        assert!(text.contains("calls: validateOrder"));

        // Embed the text
        let embedder_config = EmbedderConfig {
            dimension: 768, // nomic-embed dimension
            ..Default::default()
        };
        let mut embedder = TextEmbedder::new(embedder_config);

        let embedding = embedder.embed(&text).unwrap();

        // Should have correct dimension
        assert_eq!(embedding.len(), 768);

        // Should be unit normalized
        let norm: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 0.001);
    }
}
