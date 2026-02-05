//! Full re-embed trigger detection (I13)
//!
//! Detects when a full re-embedding of all nodes is required due to:
//! - Schema migration
//! - Config changes (embedding text config)
//! - Force flag
//! - Model changes (embedding model)

use codegraph_db::rusqlite::Connection;
use codegraph_db::QueryBuilder;
use sha2::{Digest, Sha256};

/// Metadata keys used for tracking re-embed triggers
pub mod keys {
    /// Schema version at time of last embedding
    pub const SCHEMA_VERSION: &str = "embedding_schema_version";
    /// Hash of EmbeddingTextConfig at time of last embedding
    pub const CONFIG_HASH: &str = "embedding_config_hash";
    /// Hash of embedding model at time of last embedding
    pub const MODEL_HASH: &str = "embedding_model_hash";
}

/// Reasons why a full re-embed might be required
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReembedReason {
    /// Schema version changed (migration occurred)
    SchemaMigration { old: String, new: String },
    /// Embedding text config changed
    ConfigChanged { old_hash: String, new_hash: String },
    /// User requested force re-embed
    ForceRequested,
    /// Embedding model changed
    ModelChanged { old_hash: String, new_hash: String },
    /// First-time embedding (no previous metadata)
    FirstEmbed,
}

/// Configuration for re-embed detection
pub struct ReembedConfig<'a> {
    /// Current schema version
    pub schema_version: &'a str,
    /// Serialized config for hashing
    pub config_json: &'a str,
    /// Model identifier/hash
    pub model_hash: &'a str,
    /// Whether user requested force re-embed
    pub force: bool,
}

/// Check if full re-embedding is required
///
/// Returns a list of reasons if re-embedding is needed, or empty if incremental is OK.
pub fn check_reembed_triggers(
    conn: &Connection,
    queries: &QueryBuilder,
    config: &ReembedConfig<'_>,
) -> Vec<ReembedReason> {
    let mut reasons = Vec::new();

    // Check force flag first
    if config.force {
        reasons.push(ReembedReason::ForceRequested);
        return reasons;
    }

    // Check schema version
    match queries.get_metadata(conn, keys::SCHEMA_VERSION) {
        Ok(Some(old_version)) if old_version != config.schema_version => {
            reasons.push(ReembedReason::SchemaMigration {
                old: old_version,
                new: config.schema_version.to_string(),
            });
        }
        Ok(None) => {
            // No previous schema version - first embed
            reasons.push(ReembedReason::FirstEmbed);
            return reasons;
        }
        _ => {}
    }

    // Check config hash
    let new_config_hash = compute_hash(config.config_json);
    match queries.get_metadata(conn, keys::CONFIG_HASH) {
        Ok(Some(old_hash)) if old_hash != new_config_hash => {
            reasons.push(ReembedReason::ConfigChanged {
                old_hash,
                new_hash: new_config_hash,
            });
        }
        Ok(None) => {
            // No previous config - first embed
            if !reasons.contains(&ReembedReason::FirstEmbed) {
                reasons.push(ReembedReason::FirstEmbed);
            }
        }
        _ => {}
    }

    // Check model hash
    match queries.get_metadata(conn, keys::MODEL_HASH) {
        Ok(Some(old_hash)) if old_hash != config.model_hash => {
            reasons.push(ReembedReason::ModelChanged {
                old_hash,
                new_hash: config.model_hash.to_string(),
            });
        }
        Ok(None) => {
            // No previous model - first embed
            if !reasons.contains(&ReembedReason::FirstEmbed) {
                reasons.push(ReembedReason::FirstEmbed);
            }
        }
        _ => {}
    }

    reasons
}

/// Check if re-embedding is required (convenience function)
pub fn should_full_reembed(
    conn: &Connection,
    queries: &QueryBuilder,
    config: &ReembedConfig<'_>,
) -> bool {
    !check_reembed_triggers(conn, queries, config).is_empty()
}

/// Record that embedding was completed with current config
///
/// Call this after a successful full or incremental embed to update metadata.
pub fn record_embed_metadata(
    conn: &Connection,
    queries: &QueryBuilder,
    config: &ReembedConfig<'_>,
) -> Result<(), codegraph_db::DbError> {
    queries.set_metadata(conn, keys::SCHEMA_VERSION, config.schema_version)?;
    queries.set_metadata(conn, keys::CONFIG_HASH, &compute_hash(config.config_json))?;
    queries.set_metadata(conn, keys::MODEL_HASH, config.model_hash)?;
    Ok(())
}

/// Compute SHA256 hash of content (first 16 chars of hex)
fn compute_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let result = hasher.finalize();
    hex::encode(&result[..8]) // First 8 bytes = 16 hex chars
}

#[cfg(test)]
mod tests {
    use super::*;
    use codegraph_db::{DatabaseConnection, run_migrations};

    fn setup_test_db() -> (DatabaseConnection, QueryBuilder) {
        let db = DatabaseConnection::open_in_memory().unwrap();
        run_migrations(db.conn()).unwrap();
        let queries = QueryBuilder::new(db.conn()).unwrap();
        (db, queries)
    }

    #[test]
    fn test_first_embed_detected() {
        let (db, queries) = setup_test_db();

        let config = ReembedConfig {
            schema_version: "1",
            config_json: "{}",
            model_hash: "abc123",
            force: false,
        };

        let reasons = check_reembed_triggers(db.conn(), &queries, &config);
        assert!(reasons.contains(&ReembedReason::FirstEmbed));
    }

    #[test]
    fn test_schema_migration_triggers_reembed() {
        let (db, queries) = setup_test_db();

        // Set up old version
        queries.set_metadata(db.conn(), keys::SCHEMA_VERSION, "1").unwrap();
        queries.set_metadata(db.conn(), keys::CONFIG_HASH, "oldhash").unwrap();
        queries.set_metadata(db.conn(), keys::MODEL_HASH, "modelhash").unwrap();

        let config = ReembedConfig {
            schema_version: "2", // Changed
            config_json: "{}",
            model_hash: "modelhash",
            force: false,
        };

        let reasons = check_reembed_triggers(db.conn(), &queries, &config);
        assert!(reasons.iter().any(|r| matches!(r, ReembedReason::SchemaMigration { .. })));
    }

    #[test]
    fn test_config_change_triggers_reembed() {
        let (db, queries) = setup_test_db();

        // Set up initial state
        let old_config_json = r#"{"max_tokens": 2000}"#;
        queries.set_metadata(db.conn(), keys::SCHEMA_VERSION, "1").unwrap();
        queries.set_metadata(db.conn(), keys::CONFIG_HASH, &compute_hash(old_config_json)).unwrap();
        queries.set_metadata(db.conn(), keys::MODEL_HASH, "modelhash").unwrap();

        let new_config_json = r#"{"max_tokens": 3000}"#; // Changed
        let config = ReembedConfig {
            schema_version: "1",
            config_json: new_config_json,
            model_hash: "modelhash",
            force: false,
        };

        let reasons = check_reembed_triggers(db.conn(), &queries, &config);
        assert!(reasons.iter().any(|r| matches!(r, ReembedReason::ConfigChanged { .. })));
    }

    #[test]
    fn test_force_flag_triggers_reembed() {
        let (db, queries) = setup_test_db();

        // Set up fully populated state
        queries.set_metadata(db.conn(), keys::SCHEMA_VERSION, "1").unwrap();
        queries.set_metadata(db.conn(), keys::CONFIG_HASH, "hash").unwrap();
        queries.set_metadata(db.conn(), keys::MODEL_HASH, "model").unwrap();

        let config = ReembedConfig {
            schema_version: "1",
            config_json: "same",
            model_hash: "model",
            force: true, // Force re-embed
        };

        let reasons = check_reembed_triggers(db.conn(), &queries, &config);
        assert!(reasons.contains(&ReembedReason::ForceRequested));
    }

    #[test]
    fn test_model_change_triggers_reembed() {
        let (db, queries) = setup_test_db();

        queries.set_metadata(db.conn(), keys::SCHEMA_VERSION, "1").unwrap();
        queries.set_metadata(db.conn(), keys::CONFIG_HASH, &compute_hash("{}")).unwrap();
        queries.set_metadata(db.conn(), keys::MODEL_HASH, "old-model").unwrap();

        let config = ReembedConfig {
            schema_version: "1",
            config_json: "{}",
            model_hash: "new-model", // Changed
            force: false,
        };

        let reasons = check_reembed_triggers(db.conn(), &queries, &config);
        assert!(reasons.iter().any(|r| matches!(r, ReembedReason::ModelChanged { .. })));
    }

    #[test]
    fn test_no_reembed_when_nothing_changed() {
        let (db, queries) = setup_test_db();

        let config_json = r#"{"max_tokens": 2000}"#;

        // Set up matching state
        queries.set_metadata(db.conn(), keys::SCHEMA_VERSION, "1").unwrap();
        queries.set_metadata(db.conn(), keys::CONFIG_HASH, &compute_hash(config_json)).unwrap();
        queries.set_metadata(db.conn(), keys::MODEL_HASH, "modelhash").unwrap();

        let config = ReembedConfig {
            schema_version: "1",
            config_json,
            model_hash: "modelhash",
            force: false,
        };

        let reasons = check_reembed_triggers(db.conn(), &queries, &config);
        assert!(reasons.is_empty());
        assert!(!should_full_reembed(db.conn(), &queries, &config));
    }

    #[test]
    fn test_record_embed_metadata() {
        let (db, queries) = setup_test_db();

        let config = ReembedConfig {
            schema_version: "1",
            config_json: r#"{"test": true}"#,
            model_hash: "abc123",
            force: false,
        };

        record_embed_metadata(db.conn(), &queries, &config).unwrap();

        // Verify stored
        let stored_version = queries.get_metadata(db.conn(), keys::SCHEMA_VERSION).unwrap();
        assert_eq!(stored_version, Some("1".to_string()));

        let stored_model = queries.get_metadata(db.conn(), keys::MODEL_HASH).unwrap();
        assert_eq!(stored_model, Some("abc123".to_string()));

        let stored_config = queries.get_metadata(db.conn(), keys::CONFIG_HASH).unwrap();
        assert!(stored_config.is_some());
    }
}
