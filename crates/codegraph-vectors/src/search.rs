//! Similarity search over embeddings

use crate::embedder::TextEmbedder;
use crate::error::VectorError;
use crate::storage::VectorStorage;
use rusqlite::Connection;

/// Result of a similarity search
#[derive(Debug, Clone)]
pub struct SimilarityResult {
    /// Node ID
    pub node_id: String,
    /// Similarity score (0 to 1)
    pub score: f32,
}

/// Configuration for similarity search
#[derive(Debug, Clone)]
pub struct SearchConfig {
    /// Maximum results to return
    pub max_results: usize,
    /// Minimum similarity threshold
    pub min_score: f32,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            max_results: 10,
            min_score: 0.0,
        }
    }
}

/// Similarity search over stored vectors
pub struct SimilaritySearch<'a> {
    storage: &'a VectorStorage,
    embedder: &'a mut TextEmbedder,
    config: SearchConfig,
}

impl<'a> SimilaritySearch<'a> {
    /// Create a new similarity search
    pub fn new(storage: &'a VectorStorage, embedder: &'a mut TextEmbedder) -> Self {
        Self {
            storage,
            embedder,
            config: SearchConfig::default(),
        }
    }

    /// Create with custom config
    pub fn with_config(
        storage: &'a VectorStorage,
        embedder: &'a mut TextEmbedder,
        config: SearchConfig,
    ) -> Self {
        Self {
            storage,
            embedder,
            config,
        }
    }

    /// Search for similar nodes by text query
    pub fn search_by_text(
        &mut self,
        conn: &Connection,
        query: &str,
    ) -> Result<Vec<SimilarityResult>, VectorError> {
        let query_embedding = self.embedder.embed(query)?;
        self.search_by_vector(conn, &query_embedding)
    }

    /// Search for similar nodes by vector
    pub fn search_by_vector(
        &self,
        conn: &Connection,
        query_vector: &[f32],
    ) -> Result<Vec<SimilarityResult>, VectorError> {
        let all_vectors = self.storage.get_all(conn)?;

        let mut results: Vec<SimilarityResult> = all_vectors
            .into_iter()
            .map(|record| {
                let score = cosine_similarity(query_vector, &record.vector);
                SimilarityResult {
                    node_id: record.node_id,
                    score,
                }
            })
            .filter(|r| r.score >= self.config.min_score)
            .collect();

        // Sort by score descending
        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

        // Limit results
        results.truncate(self.config.max_results);

        Ok(results)
    }

    /// Find similar nodes to a given node
    pub fn find_similar(
        &self,
        conn: &Connection,
        node_id: &str,
    ) -> Result<Vec<SimilarityResult>, VectorError> {
        let source_record = self
            .storage
            .get(conn, node_id)?
            .ok_or_else(|| VectorError::ModelNotFound {
                path: format!("Vector not found for node: {}", node_id),
            })?;

        let mut results = self.search_by_vector(conn, &source_record.vector)?;

        // Remove self from results
        results.retain(|r| r.node_id != node_id);

        Ok(results)
    }

    /// Search within a subset of nodes
    pub fn search_in_subset(
        &mut self,
        conn: &Connection,
        query: &str,
        node_ids: &[&str],
    ) -> Result<Vec<SimilarityResult>, VectorError> {
        let query_embedding = self.embedder.embed(query)?;
        let records = self.storage.get_batch(conn, node_ids)?;

        let mut results: Vec<SimilarityResult> = records
            .into_iter()
            .map(|record| {
                let score = cosine_similarity(&query_embedding, &record.vector);
                SimilarityResult {
                    node_id: record.node_id,
                    score,
                }
            })
            .filter(|r| r.score >= self.config.min_score)
            .collect();

        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(self.config.max_results);

        Ok(results)
    }
}

/// Compute cosine similarity between two vectors
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }

    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();

    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }

    dot / (norm_a * norm_b)
}

/// Compute euclidean distance between two vectors
pub fn euclidean_distance(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return f32::MAX;
    }

    a.iter()
        .zip(b.iter())
        .map(|(x, y)| (x - y).powi(2))
        .sum::<f32>()
        .sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedder::EmbedderConfig;
    use codegraph_db::DatabaseConnection;

    fn setup_test_db() -> (DatabaseConnection, VectorStorage) {
        let db = DatabaseConnection::open_in_memory().unwrap();
        let storage = VectorStorage::new(384);
        storage.init(db.conn()).unwrap();
        (db, storage)
    }

    #[test]
    fn test_cosine_similarity_identical() {
        let a = vec![1.0f32, 0.0, 0.0];
        let b = vec![1.0f32, 0.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!((sim - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let a = vec![1.0f32, 0.0, 0.0];
        let b = vec![0.0f32, 1.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!(sim.abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_opposite() {
        let a = vec![1.0f32, 0.0, 0.0];
        let b = vec![-1.0f32, 0.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!((sim - (-1.0)).abs() < 1e-6);
    }

    #[test]
    fn test_euclidean_distance() {
        let a = vec![0.0f32, 0.0, 0.0];
        let b = vec![3.0f32, 4.0, 0.0];
        let dist = euclidean_distance(&a, &b);
        assert!((dist - 5.0).abs() < 1e-6);
    }

    #[test]
    #[cfg(not(feature = "onnx"))]
    fn test_search_by_text() {
        let (db, storage) = setup_test_db();

        let config = EmbedderConfig {
            dimension: 384,
            ..Default::default()
        };
        let mut embedder = TextEmbedder::new(config);

        // Store some vectors - using exact same text for one to guarantee match
        let query_text = "function getUserById";
        let v1 = embedder.embed(query_text).unwrap();
        let v2 = embedder.embed("something completely different").unwrap();
        let v3 = embedder.embed("another unrelated thing").unwrap();

        storage.store(db.conn(), "n1", &v1, "test").unwrap();
        storage.store(db.conn(), "n2", &v2, "test").unwrap();
        storage.store(db.conn(), "n3", &v3, "test").unwrap();

        let mut search = SimilaritySearch::new(&storage, &mut embedder);
        let results = search.search_by_text(db.conn(), query_text).unwrap();

        // The most similar should be n1 (exact match text)
        assert!(!results.is_empty());
        assert_eq!(results[0].node_id, "n1");
        assert!(results[0].score > 0.99); // Very high similarity for exact match
    }

    #[test]
    #[cfg(not(feature = "onnx"))]
    fn test_find_similar() {
        let (db, storage) = setup_test_db();

        let config = EmbedderConfig {
            dimension: 384,
            ..Default::default()
        };
        let mut embedder = TextEmbedder::new(config);

        // Store some vectors
        let v1 = embedder.embed("authenticate user").unwrap();
        let v2 = embedder.embed("verify user credentials").unwrap();
        let v3 = embedder.embed("completely unrelated").unwrap();

        storage.store(db.conn(), "n1", &v1, "test").unwrap();
        storage.store(db.conn(), "n2", &v2, "test").unwrap();
        storage.store(db.conn(), "n3", &v3, "test").unwrap();

        let search = SimilaritySearch::new(&storage, &mut embedder);
        let results = search.find_similar(db.conn(), "n1").unwrap();

        // Should not include self
        assert!(results.iter().all(|r| r.node_id != "n1"));
        // Should return other nodes (at least 1, may return both)
        assert!(!results.is_empty());
    }

    #[test]
    fn test_search_config() {
        let config = SearchConfig {
            max_results: 5,
            min_score: 0.5,
        };
        assert_eq!(config.max_results, 5);
        assert!((config.min_score - 0.5).abs() < 1e-6);
    }
}
