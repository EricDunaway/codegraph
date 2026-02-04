//! Text embedding generation
//!
//! Provides embedding generation using ONNX models.
//! When the `onnx` feature is not enabled, provides a mock implementation.

use crate::error::VectorError;
use sha2::{Digest, Sha256};
use std::path::Path;

/// Configuration for the embedder
#[derive(Debug, Clone)]
pub struct EmbedderConfig {
    /// Path to the ONNX model file
    pub model_path: String,
    /// Path to the tokenizer file
    pub tokenizer_path: String,
    /// Expected SHA256 hash of the model (for integrity verification)
    pub model_hash: Option<String>,
    /// Maximum sequence length
    pub max_length: usize,
    /// Output dimension
    pub dimension: usize,
    /// Whether to use CoreML on macOS
    pub use_coreml: bool,
}

impl Default for EmbedderConfig {
    fn default() -> Self {
        Self {
            model_path: "nomic-embed-text-v1.5.onnx".to_string(),
            tokenizer_path: "tokenizer.json".to_string(),
            model_hash: None,
            max_length: 512,
            dimension: 768, // nomic-embed default
            use_coreml: cfg!(target_os = "macos"),
        }
    }
}

/// Text embedder
pub struct TextEmbedder {
    config: EmbedderConfig,
    #[cfg(feature = "onnx")]
    session: Option<ort::Session>,
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

    /// Load ONNX model (only available with onnx feature)
    #[cfg(feature = "onnx")]
    fn load_onnx(&mut self) -> Result<(), VectorError> {
        use ort::session::Session;

        // Verify model exists
        let model_path = Path::new(&self.config.model_path);
        if !model_path.exists() {
            return Err(VectorError::ModelNotFound {
                path: self.config.model_path.clone(),
            });
        }

        // Verify integrity if hash provided
        if let Some(ref expected_hash) = self.config.model_hash {
            let actual_hash = self.compute_file_hash(model_path)?;
            if actual_hash != *expected_hash {
                return Err(VectorError::IntegrityCheckFailed {
                    expected: expected_hash.clone(),
                    actual: actual_hash,
                });
            }
        }

        // Load tokenizer
        let tokenizer_path = Path::new(&self.config.tokenizer_path);
        if !tokenizer_path.exists() {
            return Err(VectorError::ModelNotFound {
                path: self.config.tokenizer_path.clone(),
            });
        }

        let tokenizer = tokenizers::Tokenizer::from_file(tokenizer_path)
            .map_err(|e| VectorError::TokenizationFailed(e.to_string()))?;

        // Build session with execution providers
        let session = {
            #[cfg(all(feature = "coreml", target_os = "macos"))]
            {
                use ort::execution_providers::CoreMLExecutionProvider;

                Session::builder()
                    .map_err(|e| VectorError::ModelLoadFailed(e.to_string()))?
                    .with_execution_providers([CoreMLExecutionProvider::default().build()])
                    .map_err(|e| VectorError::ModelLoadFailed(e.to_string()))?
                    .commit_from_file(model_path)
                    .map_err(|e| VectorError::ModelLoadFailed(e.to_string()))?
            }
            #[cfg(not(all(feature = "coreml", target_os = "macos")))]
            {
                Session::builder()
                    .map_err(|e| VectorError::ModelLoadFailed(e.to_string()))?
                    .commit_from_file(model_path)
                    .map_err(|e| VectorError::ModelLoadFailed(e.to_string()))?
            }
        };

        self.session = Some(session);
        self.tokenizer = Some(tokenizer);

        log::info!("Loaded embedding model from {}", self.config.model_path);
        Ok(())
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

    /// Generate embedding for a single text
    pub fn embed(&self, text: &str) -> Result<Vec<f32>, VectorError> {
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

    /// Generate embeddings for multiple texts
    pub fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, VectorError> {
        texts.iter().map(|t| self.embed(t)).collect()
    }

    /// ONNX embedding implementation
    #[cfg(feature = "onnx")]
    fn embed_onnx(&self, text: &str) -> Result<Vec<f32>, VectorError> {
        use ndarray::Array2;
        use ort::value::Value;

        let session = self
            .session
            .as_ref()
            .ok_or_else(|| VectorError::ModelLoadFailed("Model not loaded".to_string()))?;

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

        let seq_len = input_ids.len().min(self.config.max_length);

        // Create input tensors
        let input_ids_array = Array2::from_shape_vec((1, seq_len), input_ids[..seq_len].to_vec())
            .map_err(|e| VectorError::InferenceFailed(e.to_string()))?;

        let attention_mask_array =
            Array2::from_shape_vec((1, seq_len), attention_mask[..seq_len].to_vec())
                .map_err(|e| VectorError::InferenceFailed(e.to_string()))?;

        // Run inference
        let outputs = session
            .run(ort::inputs![
                "input_ids" => Value::from_array(input_ids_array)?,
                "attention_mask" => Value::from_array(attention_mask_array)?
            ]?)
            .map_err(|e| VectorError::InferenceFailed(e.to_string()))?;

        // Extract embedding (usually last_hidden_state or pooler_output)
        let output = outputs
            .get("last_hidden_state")
            .or_else(|| outputs.get("sentence_embedding"))
            .ok_or_else(|| VectorError::InferenceFailed("No output tensor found".to_string()))?;

        let embedding: Vec<f32> = output
            .try_extract_tensor::<f32>()
            .map_err(|e| VectorError::InferenceFailed(e.to_string()))?
            .view()
            .iter()
            .take(self.config.dimension)
            .copied()
            .collect();

        // Mean pooling if we got token embeddings
        if embedding.len() > self.config.dimension {
            let pooled = self.mean_pool(&embedding, seq_len);
            Ok(pooled)
        } else {
            Ok(embedding)
        }
    }

    /// Mean pooling over token embeddings
    #[cfg(feature = "onnx")]
    fn mean_pool(&self, embeddings: &[f32], seq_len: usize) -> Vec<f32> {
        let dim = self.config.dimension;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_embedder_config_default() {
        let config = EmbedderConfig::default();
        assert_eq!(config.dimension, 768);
        assert_eq!(config.max_length, 512);
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
        let embedder = TextEmbedder::new(config);

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
        let embedder = TextEmbedder::new(config);

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
        let embedder = TextEmbedder::new(config);

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
        let embedder = TextEmbedder::new(config);

        let texts = vec!["Hello", "World", "Test"];
        let embeddings = embedder.embed_batch(&texts).unwrap();

        assert_eq!(embeddings.len(), 3);
        for e in &embeddings {
            assert_eq!(e.len(), 384);
        }
    }
}
