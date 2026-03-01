//! Enrichment dependency tracking (I2)
//!
//! Tracks which files an LSP-enriched node depends on, enabling incremental
//! re-enrichment when dependencies change.

use crate::error::DbError;
use rusqlite::{params, Connection};

/// Insert an enrichment dependency
///
/// Records that `node_id` depends on `depends_on_file` for its LSP enrichment.
/// This is used to track which nodes need re-enrichment when a file changes.
pub fn insert_enrichment_dep(
    conn: &Connection,
    node_id: &str,
    depends_on_file: &str,
) -> Result<(), DbError> {
    conn.execute(
        "INSERT OR IGNORE INTO enrichment_deps (node_id, depends_on_file) VALUES (?1, ?2)",
        params![node_id, depends_on_file],
    )?;
    Ok(())
}

/// Insert multiple enrichment dependencies for a node
///
/// Efficient batch insert for when a node depends on multiple files.
/// Uses a transaction to ensure atomicity.
pub fn insert_enrichment_deps_batch(
    conn: &Connection,
    node_id: &str,
    depends_on_files: &[&str],
) -> Result<(), DbError> {
    if depends_on_files.is_empty() {
        return Ok(());
    }

    // Use transaction for atomicity
    conn.execute("BEGIN TRANSACTION", [])?;

    let result = (|| {
        let mut stmt = conn.prepare_cached(
            "INSERT OR IGNORE INTO enrichment_deps (node_id, depends_on_file) VALUES (?1, ?2)",
        )?;

        for file in depends_on_files {
            stmt.execute(params![node_id, *file])?;
        }

        Ok(())
    })();

    match result {
        Ok(()) => {
            conn.execute("COMMIT", [])?;
            Ok(())
        }
        Err(e) => {
            let _ = conn.execute("ROLLBACK", []);
            Err(e)
        }
    }
}

/// Get all files that a node depends on for its enrichment
pub fn get_deps_for_node(conn: &Connection, node_id: &str) -> Result<Vec<String>, DbError> {
    let mut stmt = conn.prepare_cached(
        "SELECT depends_on_file FROM enrichment_deps WHERE node_id = ?1",
    )?;

    let deps = stmt
        .query_map(params![node_id], |row| row.get(0))?
        .collect::<Result<Vec<String>, _>>()?;

    Ok(deps)
}

/// Get all nodes that depend on a specific file
///
/// When a file changes, these nodes may need re-enrichment.
pub fn get_nodes_depending_on_file(conn: &Connection, file_path: &str) -> Result<Vec<String>, DbError> {
    let mut stmt = conn.prepare_cached(
        "SELECT node_id FROM enrichment_deps WHERE depends_on_file = ?1",
    )?;

    let nodes = stmt
        .query_map(params![file_path], |row| row.get(0))?
        .collect::<Result<Vec<String>, _>>()?;

    Ok(nodes)
}

/// Maximum number of files per SQL query to prevent DoS
const MAX_FILES_PER_QUERY: usize = 500;

/// Get all nodes that depend on any of the given files
///
/// Efficient batch query for when multiple files change at once.
/// Automatically chunks large queries to prevent memory/performance issues.
pub fn get_nodes_depending_on_files(
    conn: &Connection,
    file_paths: &[&str],
) -> Result<Vec<String>, DbError> {
    if file_paths.is_empty() {
        return Ok(Vec::new());
    }

    // Chunk large queries to prevent DoS
    if file_paths.len() > MAX_FILES_PER_QUERY {
        let mut all_nodes = std::collections::HashSet::new();
        for chunk in file_paths.chunks(MAX_FILES_PER_QUERY) {
            let nodes = get_nodes_depending_on_files_inner(conn, chunk)?;
            all_nodes.extend(nodes);
        }
        return Ok(all_nodes.into_iter().collect());
    }

    get_nodes_depending_on_files_inner(conn, file_paths)
}

fn get_nodes_depending_on_files_inner(
    conn: &Connection,
    file_paths: &[&str],
) -> Result<Vec<String>, DbError> {
    // Build placeholders: (?1, ?2, ?3, ...)
    let placeholders: Vec<String> = (1..=file_paths.len()).map(|i| format!("?{}", i)).collect();
    let sql = format!(
        "SELECT DISTINCT node_id FROM enrichment_deps WHERE depends_on_file IN ({})",
        placeholders.join(", ")
    );

    let mut stmt = conn.prepare(&sql)?;
    let params: Vec<&dyn rusqlite::ToSql> = file_paths
        .iter()
        .map(|s| s as &dyn rusqlite::ToSql)
        .collect();

    let nodes = stmt
        .query_map(params.as_slice(), |row| row.get(0))?
        .collect::<Result<Vec<String>, _>>()?;

    Ok(nodes)
}

/// Clear all dependencies for a node
///
/// Called before re-enriching a node to ensure stale dependencies are removed.
pub fn clear_deps_for_node(conn: &Connection, node_id: &str) -> Result<(), DbError> {
    conn.execute(
        "DELETE FROM enrichment_deps WHERE node_id = ?1",
        params![node_id],
    )?;
    Ok(())
}

/// Clear all dependencies for multiple nodes
///
/// Efficient batch clear for when multiple nodes need re-enrichment.
/// Automatically chunks large queries to prevent memory/performance issues.
pub fn clear_deps_for_nodes(conn: &Connection, node_ids: &[&str]) -> Result<(), DbError> {
    if node_ids.is_empty() {
        return Ok(());
    }

    // Chunk large queries to prevent DoS
    if node_ids.len() > MAX_FILES_PER_QUERY {
        for chunk in node_ids.chunks(MAX_FILES_PER_QUERY) {
            clear_deps_for_nodes_inner(conn, chunk)?;
        }
        return Ok(());
    }

    clear_deps_for_nodes_inner(conn, node_ids)
}

fn clear_deps_for_nodes_inner(conn: &Connection, node_ids: &[&str]) -> Result<(), DbError> {
    let placeholders: Vec<String> = (1..=node_ids.len()).map(|i| format!("?{}", i)).collect();
    let sql = format!(
        "DELETE FROM enrichment_deps WHERE node_id IN ({})",
        placeholders.join(", ")
    );

    let mut stmt = conn.prepare(&sql)?;
    let params: Vec<&dyn rusqlite::ToSql> = node_ids
        .iter()
        .map(|s| s as &dyn rusqlite::ToSql)
        .collect();

    stmt.execute(params.as_slice())?;
    Ok(())
}

/// Clear all dependencies that reference a specific file
///
/// Called when a file is deleted from the codebase.
pub fn clear_deps_for_file(conn: &Connection, file_path: &str) -> Result<(), DbError> {
    conn.execute(
        "DELETE FROM enrichment_deps WHERE depends_on_file = ?1",
        params![file_path],
    )?;
    Ok(())
}

/// Get count of dependencies for a node
pub fn count_deps_for_node(conn: &Connection, node_id: &str) -> Result<u32, DbError> {
    let count: u32 = conn.query_row(
        "SELECT COUNT(*) FROM enrichment_deps WHERE node_id = ?1",
        params![node_id],
        |row| row.get(0),
    )?;
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrations::run_migrations;
    use crate::DatabaseConnection;

    fn setup_test_db() -> DatabaseConnection {
        let db = DatabaseConnection::open_in_memory().unwrap();
        run_migrations(db.conn()).unwrap();

        // Insert a test node (required by foreign key constraint)
        db.conn()
            .execute(
                "INSERT INTO nodes (id, kind, name, qualified_name, file_path, language,
                 start_line, end_line, start_column, end_column, updated_at)
                 VALUES ('node_1', 'function', 'test1', 'test1', 'test.rs', 'rust', 1, 1, 0, 0, 0)",
                [],
            )
            .unwrap();
        db.conn()
            .execute(
                "INSERT INTO nodes (id, kind, name, qualified_name, file_path, language,
                 start_line, end_line, start_column, end_column, updated_at)
                 VALUES ('node_2', 'function', 'test2', 'test2', 'test.rs', 'rust', 1, 1, 0, 0, 0)",
                [],
            )
            .unwrap();
        db.conn()
            .execute(
                "INSERT INTO nodes (id, kind, name, qualified_name, file_path, language,
                 start_line, end_line, start_column, end_column, updated_at)
                 VALUES ('node_3', 'function', 'test3', 'test3', 'test.rs', 'rust', 1, 1, 0, 0, 0)",
                [],
            )
            .unwrap();
        db.conn()
            .execute(
                "INSERT INTO nodes (id, kind, name, qualified_name, file_path, language,
                 start_line, end_line, start_column, end_column, updated_at)
                 VALUES ('node_123', 'function', 'test123', 'test123', 'test.rs', 'rust', 1, 1, 0, 0, 0)",
                [],
            )
            .unwrap();

        db
    }

    #[test]
    fn test_insert_enrichment_dep() {
        let db = setup_test_db();

        insert_enrichment_dep(db.conn(), "node_123", "src/utils.ts").unwrap();

        let deps = get_deps_for_node(db.conn(), "node_123").unwrap();
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0], "src/utils.ts");
    }

    #[test]
    fn test_insert_enrichment_dep_idempotent() {
        let db = setup_test_db();

        // Insert same dep twice
        insert_enrichment_dep(db.conn(), "node_1", "src/utils.ts").unwrap();
        insert_enrichment_dep(db.conn(), "node_1", "src/utils.ts").unwrap();

        let deps = get_deps_for_node(db.conn(), "node_1").unwrap();
        assert_eq!(deps.len(), 1); // Should only have one entry
    }

    #[test]
    fn test_insert_enrichment_deps_batch() {
        let db = setup_test_db();

        let files = vec!["a.ts", "b.ts", "c.ts"];
        insert_enrichment_deps_batch(db.conn(), "node_1", &files).unwrap();

        let deps = get_deps_for_node(db.conn(), "node_1").unwrap();
        assert_eq!(deps.len(), 3);
        assert!(deps.contains(&"a.ts".to_string()));
        assert!(deps.contains(&"b.ts".to_string()));
        assert!(deps.contains(&"c.ts".to_string()));
    }

    #[test]
    fn test_get_affected_nodes_by_file() {
        let db = setup_test_db();

        insert_enrichment_dep(db.conn(), "node_1", "src/types.ts").unwrap();
        insert_enrichment_dep(db.conn(), "node_2", "src/types.ts").unwrap();
        insert_enrichment_dep(db.conn(), "node_3", "src/other.ts").unwrap();

        let affected = get_nodes_depending_on_file(db.conn(), "src/types.ts").unwrap();
        assert_eq!(affected.len(), 2);
        assert!(affected.contains(&"node_1".to_string()));
        assert!(affected.contains(&"node_2".to_string()));
    }

    #[test]
    fn test_get_nodes_depending_on_files() {
        let db = setup_test_db();

        insert_enrichment_dep(db.conn(), "node_1", "a.ts").unwrap();
        insert_enrichment_dep(db.conn(), "node_2", "b.ts").unwrap();
        insert_enrichment_dep(db.conn(), "node_3", "c.ts").unwrap();

        // Query for nodes depending on a.ts or b.ts
        let affected = get_nodes_depending_on_files(db.conn(), &["a.ts", "b.ts"]).unwrap();
        assert_eq!(affected.len(), 2);
        assert!(affected.contains(&"node_1".to_string()));
        assert!(affected.contains(&"node_2".to_string()));
        assert!(!affected.contains(&"node_3".to_string()));
    }

    #[test]
    fn test_get_nodes_depending_on_files_empty() {
        let db = setup_test_db();

        let affected = get_nodes_depending_on_files(db.conn(), &[]).unwrap();
        assert!(affected.is_empty());
    }

    #[test]
    fn test_clear_deps_for_node() {
        let db = setup_test_db();

        insert_enrichment_dep(db.conn(), "node_1", "a.ts").unwrap();
        insert_enrichment_dep(db.conn(), "node_1", "b.ts").unwrap();

        clear_deps_for_node(db.conn(), "node_1").unwrap();

        let deps = get_deps_for_node(db.conn(), "node_1").unwrap();
        assert!(deps.is_empty());
    }

    #[test]
    fn test_clear_deps_for_nodes() {
        let db = setup_test_db();

        insert_enrichment_dep(db.conn(), "node_1", "a.ts").unwrap();
        insert_enrichment_dep(db.conn(), "node_2", "b.ts").unwrap();
        insert_enrichment_dep(db.conn(), "node_3", "c.ts").unwrap();

        clear_deps_for_nodes(db.conn(), &["node_1", "node_2"]).unwrap();

        assert!(get_deps_for_node(db.conn(), "node_1").unwrap().is_empty());
        assert!(get_deps_for_node(db.conn(), "node_2").unwrap().is_empty());
        assert_eq!(get_deps_for_node(db.conn(), "node_3").unwrap().len(), 1);
    }

    #[test]
    fn test_clear_deps_for_file() {
        let db = setup_test_db();

        insert_enrichment_dep(db.conn(), "node_1", "types.ts").unwrap();
        insert_enrichment_dep(db.conn(), "node_2", "types.ts").unwrap();
        insert_enrichment_dep(db.conn(), "node_3", "utils.ts").unwrap();

        clear_deps_for_file(db.conn(), "types.ts").unwrap();

        assert!(get_deps_for_node(db.conn(), "node_1").unwrap().is_empty());
        assert!(get_deps_for_node(db.conn(), "node_2").unwrap().is_empty());
        assert_eq!(get_deps_for_node(db.conn(), "node_3").unwrap().len(), 1);
    }

    #[test]
    fn test_count_deps_for_node() {
        let db = setup_test_db();

        insert_enrichment_dep(db.conn(), "node_1", "a.ts").unwrap();
        insert_enrichment_dep(db.conn(), "node_1", "b.ts").unwrap();
        insert_enrichment_dep(db.conn(), "node_1", "c.ts").unwrap();

        let count = count_deps_for_node(db.conn(), "node_1").unwrap();
        assert_eq!(count, 3);
    }

    #[test]
    fn test_count_deps_for_nonexistent_node() {
        let db = setup_test_db();

        let count = count_deps_for_node(db.conn(), "nonexistent").unwrap();
        assert_eq!(count, 0);
    }
}
