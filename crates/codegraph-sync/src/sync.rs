//! Sync manager for incremental updates

use crate::change_detector::{ChangeDetector, ChangeKind, FileChange};
use crate::error::SyncError;
use crate::lock::IndexLock;
use crate::selective::SelectiveScope;
use codegraph_db::QueryBuilder;
use codegraph_extraction::ExtractorRegistry;
use codegraph_types::{Config, EnrichmentConfig, FileRecord};
use rusqlite::Connection;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// Statistics from a sync operation
#[derive(Debug, Default, Clone)]
pub struct SyncStats {
    /// Files added
    pub files_added: usize,
    /// Files modified
    pub files_modified: usize,
    /// Files deleted
    pub files_deleted: usize,
    /// Nodes added
    pub nodes_added: usize,
    /// Nodes updated
    pub nodes_updated: usize,
    /// Nodes deleted
    pub nodes_deleted: usize,
    /// Total nodes in the graph (after sync)
    pub total_nodes: usize,
    /// Nodes needing re-enrichment (based on selective scope)
    pub nodes_to_enrich: usize,
    /// Nodes that were actually enriched
    pub nodes_enriched: usize,
    /// Errors encountered
    pub errors: Vec<String>,
}

impl SyncStats {
    /// Total files changed
    pub fn total_files_changed(&self) -> usize {
        self.files_added + self.files_modified + self.files_deleted
    }

    /// Check if any errors occurred
    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }
}

/// Result of a sync operation
#[derive(Debug)]
pub struct SyncResult {
    /// Statistics
    pub stats: SyncStats,
    /// Whether any changes were made
    pub had_changes: bool,
    /// Duration in milliseconds
    pub duration_ms: u64,
    /// Selective scope for enrichment (nodes/files needing re-enrichment)
    pub enrichment_scope: SelectiveScope,
    /// Node IDs that were removed during sync (from modified + deleted files)
    pub deleted_node_ids: Vec<String>,
    /// File paths that changed (added, modified, or deleted)
    pub changed_file_paths: Vec<String>,
}

/// Configuration for sync
#[derive(Debug, Clone)]
pub struct SyncConfig {
    /// Exclude patterns
    pub excludes: Vec<String>,
    /// Continue on error
    pub continue_on_error: bool,
    /// Batch size for processing
    pub batch_size: usize,
    /// Enrichment configuration (for cascade depth etc.)
    pub enrichment: EnrichmentConfig,
    /// Whether to use file locking
    pub use_lock: bool,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            excludes: Vec::new(),
            continue_on_error: true,
            batch_size: 100,
            enrichment: EnrichmentConfig::default(),
            use_lock: true,
        }
    }
}

/// Sync manager for incremental updates
pub struct SyncManager {
    /// Base path
    base_path: String,
    /// Configuration
    config: SyncConfig,
    /// Extractor registry
    registry: ExtractorRegistry,
}

impl SyncManager {
    /// Create a new sync manager
    pub fn new(base_path: impl Into<String>) -> Self {
        Self {
            base_path: base_path.into(),
            config: SyncConfig::default(),
            registry: ExtractorRegistry::new(),
        }
    }

    /// Create with custom configuration
    pub fn with_config(base_path: impl Into<String>, config: SyncConfig) -> Self {
        Self {
            base_path: base_path.into(),
            config,
            registry: ExtractorRegistry::new(),
        }
    }

    /// Perform a full sync
    pub fn sync(&self, conn: &Connection, queries: &mut QueryBuilder) -> Result<SyncResult, SyncError> {
        self.sync_with_codegraph_dir(conn, queries, None)
    }

    /// Perform a full sync with optional lock support
    pub fn sync_with_codegraph_dir(
        &self,
        conn: &Connection,
        queries: &mut QueryBuilder,
        codegraph_dir: Option<&Path>,
    ) -> Result<SyncResult, SyncError> {
        let start = SystemTime::now();
        let mut stats = SyncStats::default();

        // Acquire lock if configured and codegraph_dir provided
        let _lock = if self.config.use_lock {
            if let Some(dir) = codegraph_dir {
                Some(IndexLock::acquire(dir)?)
            } else {
                None
            }
        } else {
            None
        };

        // Build config for detection
        let mut detect_config = Config::default();
        if !self.config.excludes.is_empty() {
            detect_config.exclude = self.config.excludes.clone();
        }

        // Detect changes
        let detector = ChangeDetector::with_config(&self.base_path, detect_config);
        let changes = detector.detect_changes(conn, queries)?;
        let had_changes = !changes.is_empty();

        // Collect changed file paths for selective scope
        let changed_files: Vec<String> = changes.iter().map(|c| c.path.clone()).collect();

        let deleted_node_ids = if had_changes {
            // Process changes
            self.process_changes(conn, queries, &changes, &mut stats)?
        } else {
            Vec::new()
        };

        // Compute selective scope with cascade depth
        let changed_file_refs: Vec<&str> = changed_files.iter().map(|s| s.as_str()).collect();
        let enrichment_scope = SelectiveScope::from_changed_files_with_cascade(
            conn,
            &changed_file_refs,
            self.config.enrichment.cascade_depth,
        );

        // Update stats with scope info
        stats.nodes_to_enrich = enrichment_scope.node_count();

        // Get total node count
        let graph_stats = queries.get_stats(conn)?;
        stats.total_nodes = graph_stats.node_count as usize;

        let duration_ms = start
            .elapsed()
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        Ok(SyncResult {
            stats,
            had_changes,
            duration_ms,
            enrichment_scope,
            deleted_node_ids,
            changed_file_paths: changed_files,
        })
    }

    /// Sync specific files only
    pub fn sync_files(
        &self,
        conn: &Connection,
        queries: &mut QueryBuilder,
        file_paths: &[&str],
    ) -> Result<SyncResult, SyncError> {
        let start = SystemTime::now();
        let mut stats = SyncStats::default();

        let detector = ChangeDetector::new(&self.base_path);
        let changes = detector.detect_changes_for_files(conn, queries, file_paths)?;
        let had_changes = !changes.is_empty();

        // Collect changed file paths for selective scope
        let changed_files: Vec<String> = changes.iter().map(|c| c.path.clone()).collect();

        let deleted_node_ids = if had_changes {
            self.process_changes(conn, queries, &changes, &mut stats)?
        } else {
            Vec::new()
        };

        // Compute selective scope with cascade depth
        let changed_file_refs: Vec<&str> = changed_files.iter().map(|s| s.as_str()).collect();
        let enrichment_scope = SelectiveScope::from_changed_files_with_cascade(
            conn,
            &changed_file_refs,
            self.config.enrichment.cascade_depth,
        );

        // Update stats with scope info
        stats.nodes_to_enrich = enrichment_scope.node_count();

        // Get total node count
        let graph_stats = queries.get_stats(conn)?;
        stats.total_nodes = graph_stats.node_count as usize;

        let duration_ms = start
            .elapsed()
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        Ok(SyncResult {
            stats,
            had_changes,
            duration_ms,
            enrichment_scope,
            deleted_node_ids,
            changed_file_paths: changed_files,
        })
    }

    /// Process detected changes, returning IDs of deleted nodes
    fn process_changes(
        &self,
        conn: &Connection,
        queries: &mut QueryBuilder,
        changes: &[FileChange],
        stats: &mut SyncStats,
    ) -> Result<Vec<String>, SyncError> {
        let mut deleted_node_ids = Vec::new();

        for change in changes {
            match change.kind {
                ChangeKind::Added => {
                    match self.process_add(conn, queries, change) {
                        Ok(node_count) => {
                            stats.files_added += 1;
                            stats.nodes_added += node_count;
                        }
                        Err(e) => {
                            stats.errors.push(format!("Error adding {}: {}", change.path, e));
                            if !self.config.continue_on_error {
                                return Err(e);
                            }
                        }
                    }
                }
                ChangeKind::Modified => {
                    match self.process_modify(conn, queries, change) {
                        Ok((deleted, added, old_ids)) => {
                            stats.files_modified += 1;
                            stats.nodes_deleted += deleted;
                            stats.nodes_added += added;
                            deleted_node_ids.extend(old_ids);
                        }
                        Err(e) => {
                            stats.errors.push(format!("Error modifying {}: {}", change.path, e));
                            if !self.config.continue_on_error {
                                return Err(e);
                            }
                        }
                    }
                }
                ChangeKind::Deleted => {
                    match self.process_delete(conn, queries, change) {
                        Ok((node_count, old_ids)) => {
                            stats.files_deleted += 1;
                            stats.nodes_deleted += node_count;
                            deleted_node_ids.extend(old_ids);
                        }
                        Err(e) => {
                            stats.errors.push(format!("Error deleting {}: {}", change.path, e));
                            if !self.config.continue_on_error {
                                return Err(e);
                            }
                        }
                    }
                }
            }
        }

        Ok(deleted_node_ids)
    }

    /// Process an added file
    fn process_add(
        &self,
        conn: &Connection,
        queries: &mut QueryBuilder,
        change: &FileChange,
    ) -> Result<usize, SyncError> {
        let full_path = std::path::Path::new(&self.base_path).join(&change.path);
        let content = std::fs::read_to_string(&full_path)?;

        let result = self.registry.extract(&content, &change.path, change.language)?;

        // Insert nodes
        let node_count = result.nodes.len();
        queries.insert_nodes(conn, &result.nodes)?;

        // Insert edges
        queries.insert_edges(conn, &result.edges)?;

        // Insert unresolved references
        for ref_info in &result.unresolved_references {
            queries.insert_unresolved_ref(conn, ref_info)?;
        }

        // Update file record
        let file_record = FileRecord {
            path: change.path.clone(),
            content_hash: change.new_hash.clone().unwrap_or_default(),
            language: change.language,
            size: content.len() as u64,
            modified_at: now_timestamp(),
            indexed_at: now_timestamp(),
            node_count: node_count as u32,
            errors: result.errors,
        };
        queries.upsert_file(conn, &file_record)?;

        Ok(node_count)
    }

    /// Process a modified file, returning (old_count, new_count, old_node_ids)
    fn process_modify(
        &self,
        conn: &Connection,
        queries: &mut QueryBuilder,
        change: &FileChange,
    ) -> Result<(usize, usize, Vec<String>), SyncError> {
        // Capture old node IDs before deletion
        let old_nodes = queries.get_nodes_by_file(conn, &change.path)?;
        let old_count = old_nodes.len();
        let old_node_ids: Vec<String> = old_nodes.iter().map(|n| n.id.0.clone()).collect();

        // Delete old nodes and edges
        queries.delete_nodes_by_file(conn, &change.path)?;

        // Re-extract
        let full_path = std::path::Path::new(&self.base_path).join(&change.path);
        let content = std::fs::read_to_string(&full_path)?;

        let result = self.registry.extract(&content, &change.path, change.language)?;

        // Insert new nodes
        let new_count = result.nodes.len();
        queries.insert_nodes(conn, &result.nodes)?;
        queries.insert_edges(conn, &result.edges)?;

        for ref_info in &result.unresolved_references {
            queries.insert_unresolved_ref(conn, ref_info)?;
        }

        // Update file record
        let file_record = FileRecord {
            path: change.path.clone(),
            content_hash: change.new_hash.clone().unwrap_or_default(),
            language: change.language,
            size: content.len() as u64,
            modified_at: now_timestamp(),
            indexed_at: now_timestamp(),
            node_count: new_count as u32,
            errors: result.errors,
        };
        queries.upsert_file(conn, &file_record)?;

        Ok((old_count, new_count, old_node_ids))
    }

    /// Process a deleted file, returning (node_count, old_node_ids)
    fn process_delete(
        &self,
        conn: &Connection,
        queries: &mut QueryBuilder,
        change: &FileChange,
    ) -> Result<(usize, Vec<String>), SyncError> {
        // Capture old node IDs before deletion
        let old_nodes = queries.get_nodes_by_file(conn, &change.path)?;
        let count = old_nodes.len();
        let old_node_ids: Vec<String> = old_nodes.iter().map(|n| n.id.0.clone()).collect();

        // Delete file and its nodes
        queries.delete_file(conn, &change.path)?;

        Ok((count, old_node_ids))
    }
}

/// Get current timestamp
fn now_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use codegraph_db::DatabaseConnection;
    use std::fs;
    use tempfile::TempDir;

    fn setup_test_dir() -> TempDir {
        let dir = tempfile::tempdir().unwrap();

        let file = dir.path().join("test.rs");
        fs::write(&file, "fn main() { println!(\"Hello\"); }").unwrap();

        dir
    }

    #[test]
    fn test_sync_stats_default() {
        let stats = SyncStats::default();
        assert_eq!(stats.total_files_changed(), 0);
        assert!(!stats.has_errors());
    }

    #[test]
    fn test_sync_config_default() {
        let config = SyncConfig::default();
        assert!(config.excludes.is_empty());
        assert!(config.continue_on_error);
        assert_eq!(config.batch_size, 100);
    }

    #[test]
    fn test_sync_new_files() {
        let dir = setup_test_dir();
        let db = DatabaseConnection::open_in_memory().unwrap();
        let mut queries = QueryBuilder::new(db.conn()).unwrap();

        let manager = SyncManager::new(dir.path().to_str().unwrap());
        let result = manager.sync(db.conn(), &mut queries).unwrap();

        assert!(result.had_changes);
        assert_eq!(result.stats.files_added, 1);
    }

    #[test]
    fn test_sync_no_changes() {
        let dir = setup_test_dir();
        let db = DatabaseConnection::open_in_memory().unwrap();
        let mut queries = QueryBuilder::new(db.conn()).unwrap();

        let manager = SyncManager::new(dir.path().to_str().unwrap());

        // First sync
        let _ = manager.sync(db.conn(), &mut queries).unwrap();

        // Second sync should have no changes
        let result = manager.sync(db.conn(), &mut queries).unwrap();
        assert!(!result.had_changes);
    }

    #[test]
    fn test_sync_modified_file() {
        let dir = setup_test_dir();
        let db = DatabaseConnection::open_in_memory().unwrap();
        let mut queries = QueryBuilder::new(db.conn()).unwrap();

        let manager = SyncManager::new(dir.path().to_str().unwrap());

        // First sync
        let _ = manager.sync(db.conn(), &mut queries).unwrap();

        // Modify file
        let file = dir.path().join("test.rs");
        fs::write(&file, "fn main() { println!(\"Modified\"); } fn helper() {}").unwrap();

        // Second sync should detect modification
        let result = manager.sync(db.conn(), &mut queries).unwrap();
        assert!(result.had_changes);
        assert_eq!(result.stats.files_modified, 1);
    }
}
