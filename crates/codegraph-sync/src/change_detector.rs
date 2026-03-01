//! File change detection

use crate::error::SyncError;
use codegraph_db::QueryBuilder;
use codegraph_types::{Config, FileRecord, Language};
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Kind of change detected
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    /// New file added
    Added,
    /// Existing file modified
    Modified,
    /// File deleted
    Deleted,
}

/// A detected file change
#[derive(Debug, Clone)]
pub struct FileChange {
    /// Path to the file (relative)
    pub path: String,
    /// Kind of change
    pub kind: ChangeKind,
    /// Language of the file
    pub language: Language,
    /// Old content hash (if modified or deleted)
    pub old_hash: Option<String>,
    /// New content hash (if added or modified)
    pub new_hash: Option<String>,
}

/// Detects changes between the file system and the database
pub struct ChangeDetector {
    /// Base path for the project
    base_path: String,
    /// Configuration
    config: Config,
}

impl ChangeDetector {
    /// Create a new change detector
    pub fn new(base_path: impl Into<String>) -> Self {
        Self {
            base_path: base_path.into(),
            config: Config::default(),
        }
    }

    /// Create with custom configuration
    pub fn with_config(base_path: impl Into<String>, config: Config) -> Self {
        Self {
            base_path: base_path.into(),
            config,
        }
    }

    /// Detect all changes between file system and database
    pub fn detect_changes(
        &self,
        conn: &Connection,
        queries: &QueryBuilder,
    ) -> Result<Vec<FileChange>, SyncError> {
        let mut changes = Vec::new();

        // Scan current files
        let current_files = self.scan_files()?;
        let current_paths: HashMap<String, (String, Language)> = current_files
            .into_iter()
            .map(|(path, hash, lang)| (path, (hash, lang)))
            .collect();

        // Get tracked files from database
        let tracked_files = queries.get_all_files(conn)?;
        let tracked_paths: HashMap<String, FileRecord> = tracked_files
            .into_iter()
            .map(|f| (f.path.clone(), f))
            .collect();

        // Find added and modified files
        for (path, (hash, language)) in &current_paths {
            if let Some(tracked) = tracked_paths.get(path) {
                // File exists in both - check if modified
                if tracked.content_hash != *hash {
                    changes.push(FileChange {
                        path: path.clone(),
                        kind: ChangeKind::Modified,
                        language: *language,
                        old_hash: Some(tracked.content_hash.clone()),
                        new_hash: Some(hash.clone()),
                    });
                }
            } else {
                // New file
                changes.push(FileChange {
                    path: path.clone(),
                    kind: ChangeKind::Added,
                    language: *language,
                    old_hash: None,
                    new_hash: Some(hash.clone()),
                });
            }
        }

        // Find deleted files
        for (path, tracked) in &tracked_paths {
            if !current_paths.contains_key(path) {
                changes.push(FileChange {
                    path: path.clone(),
                    kind: ChangeKind::Deleted,
                    language: tracked.language,
                    old_hash: Some(tracked.content_hash.clone()),
                    new_hash: None,
                });
            }
        }

        Ok(changes)
    }

    /// Detect changes for specific files only
    pub fn detect_changes_for_files(
        &self,
        conn: &Connection,
        queries: &QueryBuilder,
        file_paths: &[&str],
    ) -> Result<Vec<FileChange>, SyncError> {
        let mut changes = Vec::new();

        for &path in file_paths {
            let full_path = Path::new(&self.base_path).join(path);
            let tracked = queries.get_file_by_path(conn, path)?;

            if full_path.exists() {
                // File exists on disk
                let content = fs::read_to_string(&full_path)?;
                let hash = compute_hash(&content);
                let language = Language::from_extension(
                    full_path.extension().and_then(|s| s.to_str()).unwrap_or("")
                );

                if let Some(ref tr) = tracked {
                    // Check if modified
                    if tr.content_hash != hash {
                        changes.push(FileChange {
                            path: path.to_string(),
                            kind: ChangeKind::Modified,
                            language,
                            old_hash: Some(tr.content_hash.clone()),
                            new_hash: Some(hash),
                        });
                    }
                } else {
                    // New file
                    changes.push(FileChange {
                        path: path.to_string(),
                        kind: ChangeKind::Added,
                        language,
                        old_hash: None,
                        new_hash: Some(hash),
                    });
                }
            } else if let Some(ref tr) = tracked {
                // File deleted
                changes.push(FileChange {
                    path: path.to_string(),
                    kind: ChangeKind::Deleted,
                    language: tr.language,
                    old_hash: Some(tr.content_hash.clone()),
                    new_hash: None,
                });
            }
        }

        Ok(changes)
    }

    /// Scan files in the base path
    fn scan_files(&self) -> Result<Vec<(String, String, Language)>, SyncError> {
        let mut results = Vec::new();
        let base = Path::new(&self.base_path);

        self.scan_dir(base, base, &mut results)?;

        Ok(results)
    }

    /// Recursively scan a directory
    fn scan_dir(
        &self,
        dir: &Path,
        base: &Path,
        results: &mut Vec<(String, String, Language)>,
    ) -> Result<(), SyncError> {
        if !dir.exists() || !dir.is_dir() {
            return Ok(());
        }

        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            let rel_path = path.strip_prefix(base).unwrap_or(&path);
            let rel_str = rel_path.to_string_lossy().to_string();

            // Skip excluded directories
            if path.is_dir() {
                if self.should_exclude(&rel_str) {
                    continue;
                }
                self.scan_dir(&path, base, results)?;
            } else if path.is_file() {
                if self.should_exclude(&rel_str) {
                    continue;
                }

                // Check if it's a source file
                let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
                let lang = Language::from_extension(ext);

                if lang != Language::Unknown {
                    let content = fs::read_to_string(&path)?;
                    let hash = compute_hash(&content);
                    results.push((rel_str, hash, lang));
                }
            }
        }

        Ok(())
    }

    /// Check if a path should be excluded
    fn should_exclude(&self, path: &str) -> bool {
        // Simple check for common excludes
        for pattern in &self.config.exclude {
            if pattern.ends_with("/**") {
                let prefix = &pattern[..pattern.len() - 3];
                if path.starts_with(prefix) || path.contains(&format!("/{}/", prefix.trim_start_matches("**/"))) {
                    return true;
                }
            } else if path.contains(pattern.trim_matches('*')) {
                return true;
            }
        }
        false
    }

    /// Get a summary of changes
    pub fn summarize_changes(changes: &[FileChange]) -> (usize, usize, usize) {
        let added = changes.iter().filter(|c| c.kind == ChangeKind::Added).count();
        let modified = changes.iter().filter(|c| c.kind == ChangeKind::Modified).count();
        let deleted = changes.iter().filter(|c| c.kind == ChangeKind::Deleted).count();
        (added, modified, deleted)
    }
}

/// Compute SHA256 hash of content
pub fn compute_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use codegraph_db::DatabaseConnection;
    use tempfile::TempDir;

    fn setup_test_dir() -> TempDir {
        let dir = tempfile::tempdir().unwrap();

        // Create test files
        let file1 = dir.path().join("test.rs");
        fs::write(&file1, "fn main() {}").unwrap();

        let file2 = dir.path().join("lib.rs");
        fs::write(&file2, "pub fn hello() {}").unwrap();

        dir
    }

    #[test]
    fn test_detect_new_files() {
        let dir = setup_test_dir();
        let db = DatabaseConnection::open_in_memory().unwrap();
        let queries = QueryBuilder::new(db.conn()).unwrap();

        let detector = ChangeDetector::new(dir.path().to_str().unwrap());
        let changes = detector.detect_changes(db.conn(), &queries).unwrap();

        // All files should be new
        assert_eq!(changes.len(), 2);
        assert!(changes.iter().all(|c| c.kind == ChangeKind::Added));
    }

    #[test]
    fn test_detect_modified_file() {
        let dir = setup_test_dir();
        let db = DatabaseConnection::open_in_memory().unwrap();
        let queries = QueryBuilder::new(db.conn()).unwrap();

        // Track the file first
        let record = FileRecord {
            path: "test.rs".to_string(),
            content_hash: "old_hash".to_string(),
            language: Language::Rust,
            size: 10,
            modified_at: 0,
            indexed_at: 0,
            node_count: 0,
            errors: Vec::new(),
        };
        queries.upsert_file(db.conn(), &record).unwrap();

        let detector = ChangeDetector::new(dir.path().to_str().unwrap());
        let changes = detector.detect_changes(db.conn(), &queries).unwrap();

        // test.rs should be modified (hash changed), lib.rs should be new
        let modified: Vec<_> = changes.iter().filter(|c| c.kind == ChangeKind::Modified).collect();
        let added: Vec<_> = changes.iter().filter(|c| c.kind == ChangeKind::Added).collect();

        assert_eq!(modified.len(), 1);
        assert_eq!(modified[0].path, "test.rs");
        assert_eq!(added.len(), 1);
    }

    #[test]
    fn test_detect_deleted_file() {
        let dir = setup_test_dir();
        let db = DatabaseConnection::open_in_memory().unwrap();
        let queries = QueryBuilder::new(db.conn()).unwrap();

        // Track a file that doesn't exist
        let record = FileRecord {
            path: "deleted.rs".to_string(),
            content_hash: "some_hash".to_string(),
            language: Language::Rust,
            size: 10,
            modified_at: 0,
            indexed_at: 0,
            node_count: 0,
            errors: Vec::new(),
        };
        queries.upsert_file(db.conn(), &record).unwrap();

        let detector = ChangeDetector::new(dir.path().to_str().unwrap());
        let changes = detector.detect_changes(db.conn(), &queries).unwrap();

        let deleted: Vec<_> = changes.iter().filter(|c| c.kind == ChangeKind::Deleted).collect();
        assert_eq!(deleted.len(), 1);
        assert_eq!(deleted[0].path, "deleted.rs");
    }

    #[test]
    fn test_summarize_changes() {
        let changes = vec![
            FileChange {
                path: "a.rs".to_string(),
                kind: ChangeKind::Added,
                language: Language::Rust,
                old_hash: None,
                new_hash: Some("hash".to_string()),
            },
            FileChange {
                path: "b.rs".to_string(),
                kind: ChangeKind::Added,
                language: Language::Rust,
                old_hash: None,
                new_hash: Some("hash".to_string()),
            },
            FileChange {
                path: "c.rs".to_string(),
                kind: ChangeKind::Modified,
                language: Language::Rust,
                old_hash: Some("old".to_string()),
                new_hash: Some("new".to_string()),
            },
            FileChange {
                path: "d.rs".to_string(),
                kind: ChangeKind::Deleted,
                language: Language::Rust,
                old_hash: Some("hash".to_string()),
                new_hash: None,
            },
        ];

        let (added, modified, deleted) = ChangeDetector::summarize_changes(&changes);
        assert_eq!(added, 2);
        assert_eq!(modified, 1);
        assert_eq!(deleted, 1);
    }
}
