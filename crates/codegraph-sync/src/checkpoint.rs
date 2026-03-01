//! Checkpoint management for sync.last_head.
//!
//! Reads and writes the last known git HEAD SHA so that incremental sync
//! can determine what changed since the previous run.

use codegraph_db::QueryBuilder;
use rusqlite::Connection;
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

pub const LAST_HEAD_KEY: &str = "sync.last_head";
pub const LAST_TIMESTAMP_KEY: &str = "sync.last_timestamp";

/// Read the last synced HEAD SHA from the metadata table.
pub fn read_last_head(
    conn: &Connection,
    queries: &QueryBuilder,
) -> Result<Option<String>, codegraph_db::DbError> {
    queries.get_metadata(conn, LAST_HEAD_KEY)
}

/// Write the current HEAD SHA into the metadata table.
pub fn write_last_head(
    conn: &Connection,
    queries: &QueryBuilder,
    head: &str,
) -> Result<(), codegraph_db::DbError> {
    queries.set_metadata(conn, LAST_HEAD_KEY, head)
}

/// Write the current unix timestamp into `sync.last_timestamp`.
pub fn write_last_timestamp(
    conn: &Connection,
    queries: &QueryBuilder,
) -> Result<(), codegraph_db::DbError> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .to_string();
    queries.set_metadata(conn, LAST_TIMESTAMP_KEY, &now)
}

/// Get current git HEAD SHA. Returns `None` if not in a git repo.
pub fn get_git_head(repo_root: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo_root)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let head = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if head.is_empty() {
        None
    } else {
        Some(head)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codegraph_db::DatabaseConnection;
    use tempfile::TempDir;

    #[test]
    fn test_read_write_last_head_roundtrip() {
        let db = DatabaseConnection::open_in_memory().unwrap();
        let queries = QueryBuilder::new(db.conn()).unwrap();

        // Initially there should be no last_head
        let result = read_last_head(db.conn(), &queries).unwrap();
        assert!(result.is_none());

        // Write a HEAD SHA
        let sha = "abc123def456";
        write_last_head(db.conn(), &queries, sha).unwrap();

        // Read it back
        let result = read_last_head(db.conn(), &queries).unwrap();
        assert_eq!(result, Some(sha.to_string()));

        // Overwrite with a new SHA
        let sha2 = "789xyz000111";
        write_last_head(db.conn(), &queries, sha2).unwrap();

        let result = read_last_head(db.conn(), &queries).unwrap();
        assert_eq!(result, Some(sha2.to_string()));
    }

    #[test]
    fn test_write_last_timestamp() {
        let db = DatabaseConnection::open_in_memory().unwrap();
        let queries = QueryBuilder::new(db.conn()).unwrap();

        write_last_timestamp(db.conn(), &queries).unwrap();

        let ts = queries
            .get_metadata(db.conn(), LAST_TIMESTAMP_KEY)
            .unwrap();
        assert!(ts.is_some());
        // The timestamp should be a valid integer
        let ts_val: u64 = ts.unwrap().parse().unwrap();
        assert!(ts_val > 0);
    }

    #[test]
    fn test_get_git_head_returns_none_in_non_repo() {
        let temp = TempDir::new().unwrap();
        let result = get_git_head(temp.path());
        assert!(result.is_none());
    }
}
