//! Vector storage in SQLite

use crate::error::VectorError;
use rusqlite::{params, Connection, OptionalExtension};
use std::time::{SystemTime, UNIX_EPOCH};

/// Stored vector record
#[derive(Debug, Clone)]
pub struct VectorRecord {
    /// Node ID this vector belongs to
    pub node_id: String,
    /// The embedding vector
    pub vector: Vec<f32>,
    /// Model name used to generate this vector
    pub model: String,
    /// When the vector was created
    pub created_at: i64,
}

/// Vector storage manager
pub struct VectorStorage {
    /// Vector dimension
    dimension: usize,
}

impl VectorStorage {
    /// Create a new vector storage manager
    pub fn new(dimension: usize) -> Self {
        Self { dimension }
    }

    /// Initialize the vectors table if it doesn't exist
    pub fn init(&self, conn: &Connection) -> Result<(), VectorError> {
        conn.execute(
            r#"
            CREATE TABLE IF NOT EXISTS vectors (
                node_id TEXT PRIMARY KEY,
                vector BLOB NOT NULL,
                model TEXT NOT NULL,
                dimension INTEGER NOT NULL,
                created_at INTEGER NOT NULL
            )
            "#,
            [],
        )?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_vectors_model ON vectors(model)",
            [],
        )?;

        Ok(())
    }

    /// Store a vector for a node
    pub fn store(
        &self,
        conn: &Connection,
        node_id: &str,
        vector: &[f32],
        model: &str,
    ) -> Result<(), VectorError> {
        if vector.len() != self.dimension {
            return Err(VectorError::DimensionMismatch {
                expected: self.dimension,
                actual: vector.len(),
            });
        }

        let blob = vector_to_blob(vector);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        conn.execute(
            r#"
            INSERT INTO vectors (node_id, vector, model, dimension, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5)
            ON CONFLICT(node_id) DO UPDATE SET
                vector = ?2,
                model = ?3,
                dimension = ?4,
                created_at = ?5
            "#,
            params![node_id, blob, model, self.dimension as i64, now],
        )?;

        Ok(())
    }

    /// Get a vector for a node
    pub fn get(&self, conn: &Connection, node_id: &str) -> Result<Option<VectorRecord>, VectorError> {
        let result = conn
            .query_row(
                "SELECT node_id, vector, model, created_at FROM vectors WHERE node_id = ?",
                params![node_id],
                |row| {
                    let blob: Vec<u8> = row.get("vector")?;
                    Ok(VectorRecord {
                        node_id: row.get("node_id")?,
                        vector: blob_to_vector(&blob),
                        model: row.get("model")?,
                        created_at: row.get("created_at")?,
                    })
                },
            )
            .optional()?;

        Ok(result)
    }

    /// Delete a vector for a node
    pub fn delete(&self, conn: &Connection, node_id: &str) -> Result<(), VectorError> {
        conn.execute("DELETE FROM vectors WHERE node_id = ?", params![node_id])?;
        Ok(())
    }

    /// Delete vectors for multiple nodes in a single query
    pub fn delete_batch(&self, conn: &Connection, node_ids: &[&str]) -> Result<(), VectorError> {
        if node_ids.is_empty() {
            return Ok(());
        }

        let placeholders: String = node_ids
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 1))
            .collect::<Vec<_>>()
            .join(",");

        let sql = format!("DELETE FROM vectors WHERE node_id IN ({})", placeholders);
        let mut stmt = conn.prepare(&sql)?;
        let params: Vec<Box<dyn rusqlite::ToSql>> = node_ids
            .iter()
            .map(|id| Box::new(id.to_string()) as Box<dyn rusqlite::ToSql>)
            .collect();
        let params_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        stmt.execute(params_refs.as_slice())?;

        Ok(())
    }

    /// Get all vectors (for similarity search)
    pub fn get_all(&self, conn: &Connection) -> Result<Vec<VectorRecord>, VectorError> {
        let mut stmt = conn.prepare(
            "SELECT node_id, vector, model, created_at FROM vectors"
        )?;

        let records = stmt
            .query_map([], |row| {
                let blob: Vec<u8> = row.get("vector")?;
                Ok(VectorRecord {
                    node_id: row.get("node_id")?,
                    vector: blob_to_vector(&blob),
                    model: row.get("model")?,
                    created_at: row.get("created_at")?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(records)
    }

    /// Get vectors for specific nodes
    pub fn get_batch(
        &self,
        conn: &Connection,
        node_ids: &[&str],
    ) -> Result<Vec<VectorRecord>, VectorError> {
        if node_ids.is_empty() {
            return Ok(Vec::new());
        }

        let placeholders: String = node_ids
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 1))
            .collect::<Vec<_>>()
            .join(",");

        let sql = format!(
            "SELECT node_id, vector, model, created_at FROM vectors WHERE node_id IN ({})",
            placeholders
        );

        let mut stmt = conn.prepare(&sql)?;

        let params: Vec<Box<dyn rusqlite::ToSql>> = node_ids
            .iter()
            .map(|id| Box::new(id.to_string()) as Box<dyn rusqlite::ToSql>)
            .collect();

        let params_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();

        let records = stmt
            .query_map(params_refs.as_slice(), |row| {
                let blob: Vec<u8> = row.get("vector")?;
                Ok(VectorRecord {
                    node_id: row.get("node_id")?,
                    vector: blob_to_vector(&blob),
                    model: row.get("model")?,
                    created_at: row.get("created_at")?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(records)
    }

    /// Count total vectors
    pub fn count(&self, conn: &Connection) -> Result<u64, VectorError> {
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM vectors", [], |row| row.get(0))?;
        Ok(count as u64)
    }

    /// Clear all vectors
    pub fn clear(&self, conn: &Connection) -> Result<(), VectorError> {
        conn.execute("DELETE FROM vectors", [])?;
        Ok(())
    }
}

/// Convert a vector to a blob for storage
fn vector_to_blob(vector: &[f32]) -> Vec<u8> {
    let mut blob = Vec::with_capacity(vector.len() * 4);
    for &v in vector {
        blob.extend_from_slice(&v.to_le_bytes());
    }
    blob
}

/// Convert a blob back to a vector
fn blob_to_vector(blob: &[u8]) -> Vec<f32> {
    let mut vector = Vec::with_capacity(blob.len() / 4);
    for chunk in blob.chunks_exact(4) {
        let bytes: [u8; 4] = chunk.try_into().unwrap();
        vector.push(f32::from_le_bytes(bytes));
    }
    vector
}

#[cfg(test)]
mod tests {
    use super::*;
    use codegraph_db::DatabaseConnection;

    #[test]
    fn test_vector_to_blob_roundtrip() {
        let vector = vec![1.0f32, 2.5, -3.7, 0.0, 1e-6];
        let blob = vector_to_blob(&vector);
        let restored = blob_to_vector(&blob);

        assert_eq!(vector.len(), restored.len());
        for (a, b) in vector.iter().zip(restored.iter()) {
            assert!((a - b).abs() < 1e-9);
        }
    }

    #[test]
    fn test_store_and_get_vector() {
        let db = DatabaseConnection::open_in_memory().unwrap();
        let storage = VectorStorage::new(384);
        storage.init(db.conn()).unwrap();

        let vector: Vec<f32> = (0..384).map(|i| i as f32 / 384.0).collect();
        storage
            .store(db.conn(), "node1", &vector, "test-model")
            .unwrap();

        let record = storage.get(db.conn(), "node1").unwrap();
        assert!(record.is_some());

        let record = record.unwrap();
        assert_eq!(record.node_id, "node1");
        assert_eq!(record.model, "test-model");
        assert_eq!(record.vector.len(), 384);
    }

    #[test]
    fn test_dimension_mismatch() {
        let db = DatabaseConnection::open_in_memory().unwrap();
        let storage = VectorStorage::new(384);
        storage.init(db.conn()).unwrap();

        let wrong_size = vec![1.0f32; 256];
        let result = storage.store(db.conn(), "node1", &wrong_size, "test");

        assert!(matches!(
            result,
            Err(VectorError::DimensionMismatch { .. })
        ));
    }

    #[test]
    fn test_delete_vector() {
        let db = DatabaseConnection::open_in_memory().unwrap();
        let storage = VectorStorage::new(384);
        storage.init(db.conn()).unwrap();

        let vector: Vec<f32> = vec![0.0; 384];
        storage
            .store(db.conn(), "node1", &vector, "test")
            .unwrap();

        assert!(storage.get(db.conn(), "node1").unwrap().is_some());

        storage.delete(db.conn(), "node1").unwrap();

        assert!(storage.get(db.conn(), "node1").unwrap().is_none());
    }

    #[test]
    fn test_count_vectors() {
        let db = DatabaseConnection::open_in_memory().unwrap();
        let storage = VectorStorage::new(384);
        storage.init(db.conn()).unwrap();

        let vector: Vec<f32> = vec![0.0; 384];

        assert_eq!(storage.count(db.conn()).unwrap(), 0);

        storage.store(db.conn(), "n1", &vector, "test").unwrap();
        storage.store(db.conn(), "n2", &vector, "test").unwrap();
        storage.store(db.conn(), "n3", &vector, "test").unwrap();

        assert_eq!(storage.count(db.conn()).unwrap(), 3);
    }

    #[test]
    fn test_delete_batch_vectors() {
        let db = DatabaseConnection::open_in_memory().unwrap();
        let storage = VectorStorage::new(384);
        storage.init(db.conn()).unwrap();

        let vector: Vec<f32> = vec![0.0; 384];
        storage.store(db.conn(), "n1", &vector, "test").unwrap();
        storage.store(db.conn(), "n2", &vector, "test").unwrap();
        storage.store(db.conn(), "n3", &vector, "test").unwrap();

        assert_eq!(storage.count(db.conn()).unwrap(), 3);

        storage.delete_batch(db.conn(), &["n1", "n3"]).unwrap();

        assert_eq!(storage.count(db.conn()).unwrap(), 1);
        assert!(storage.get(db.conn(), "n1").unwrap().is_none());
        assert!(storage.get(db.conn(), "n2").unwrap().is_some());
        assert!(storage.get(db.conn(), "n3").unwrap().is_none());
    }

    #[test]
    fn test_delete_batch_empty() {
        let db = DatabaseConnection::open_in_memory().unwrap();
        let storage = VectorStorage::new(384);
        storage.init(db.conn()).unwrap();

        // Should not error on empty input
        storage.delete_batch(db.conn(), &[]).unwrap();
    }
}
