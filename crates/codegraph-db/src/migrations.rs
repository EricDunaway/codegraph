//! Database schema migrations

use crate::error::DbError;
use rusqlite::Connection;

/// Get current schema version
pub fn get_schema_version(conn: &Connection) -> Result<u32, DbError> {
    let version: u32 = conn.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_version",
        [],
        |row| row.get(0),
    )?;
    Ok(version)
}

/// Migrate schema from v1 to v2 (add enrichment columns)
pub fn migrate_to_v2(conn: &Connection) -> Result<(), DbError> {
    let current = get_schema_version(conn)?;
    if current >= 2 {
        return Ok(()); // Already migrated
    }

    // Use transaction for atomicity
    conn.execute_batch(
        r#"
        BEGIN TRANSACTION;

        -- LSP enrichment fields
        ALTER TABLE nodes ADD COLUMN inferred_type TEXT;
        ALTER TABLE nodes ADD COLUMN resolved_import_path TEXT;

        -- New extraction fields
        ALTER TABLE nodes ADD COLUMN code_snippet TEXT;
        ALTER TABLE nodes ADD COLUMN thrown_errors TEXT DEFAULT '[]';
        ALTER TABLE nodes ADD COLUMN test_names TEXT DEFAULT '[]';
        ALTER TABLE nodes ADD COLUMN package_name TEXT;

        -- Dependency tracking for incremental LSP updates
        CREATE TABLE IF NOT EXISTS enrichment_deps (
            node_id TEXT NOT NULL,
            depends_on_file TEXT NOT NULL,
            PRIMARY KEY (node_id, depends_on_file),
            FOREIGN KEY (node_id) REFERENCES nodes(id) ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS idx_enrichment_deps_file ON enrichment_deps(depends_on_file);

        -- Metadata for version tracking
        CREATE TABLE IF NOT EXISTS metadata (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );

        -- Record migration
        INSERT INTO schema_version (version, applied_at, description)
        VALUES (2, strftime('%s', 'now'), 'Add enrichment columns');

        COMMIT;
    "#,
    )?;

    Ok(())
}

/// Migrate schema from v2 to v3 (edge dedup + resolved refs)
pub fn migrate_to_v3(conn: &Connection) -> Result<(), DbError> {
    let current = get_schema_version(conn)?;
    if current >= 3 {
        return Ok(());
    }

    conn.execute_batch(
        r#"
        BEGIN TRANSACTION;

        -- Deduplicate existing edge rows
        DELETE FROM edges WHERE rowid NOT IN (
            SELECT MIN(rowid) FROM edges
            GROUP BY source, target, kind, COALESCE(line, -1), COALESCE(col, -1)
        );

        -- Add unique index to prevent future duplicates
        CREATE UNIQUE INDEX IF NOT EXISTS idx_edges_unique
        ON edges(source, target, kind, COALESCE(line, -1), COALESCE(col, -1));

        -- Add resolved column to unresolved_refs (soft delete)
        ALTER TABLE unresolved_refs ADD COLUMN resolved INTEGER DEFAULT 0;

        -- Index for scoped resolution queries
        CREATE INDEX IF NOT EXISTS idx_unresolved_name_resolved
        ON unresolved_refs(reference_name, resolved);

        -- Record migration
        INSERT INTO schema_version (version, applied_at, description)
        VALUES (3, strftime('%s', 'now'), 'Edge dedup + unresolved_refs retention');

        COMMIT;
    "#,
    )?;

    Ok(())
}

/// Run all pending migrations
pub fn run_migrations(conn: &Connection) -> Result<(), DbError> {
    let current = get_schema_version(conn)?;

    if current < 2 {
        migrate_to_v2(conn)?;
    }

    if current < 3 {
        migrate_to_v3(conn)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DatabaseConnection;

    #[test]
    fn test_get_schema_version_after_migrations() {
        let db = DatabaseConnection::open_in_memory().unwrap();
        // Migrations auto-run on connection open, so we should be at v3
        let version = get_schema_version(db.conn()).unwrap();
        assert_eq!(version, 3);
    }

    #[test]
    fn test_migration_to_v2() {
        let db = DatabaseConnection::open_in_memory().unwrap();

        // Run migration (idempotent - already applied by open_in_memory)
        migrate_to_v2(db.conn()).unwrap();

        // Version is at latest (v3) since open_in_memory runs all migrations
        let version = get_schema_version(db.conn()).unwrap();
        assert!(version >= 2);

        // Verify new columns exist by inserting with them
        db.conn()
            .execute(
                "INSERT INTO nodes (id, kind, name, qualified_name, file_path, language,
                 start_line, end_line, start_column, end_column, updated_at,
                 inferred_type, resolved_import_path, code_snippet, thrown_errors,
                 test_names, package_name)
                 VALUES ('test', 'function', 'test', 'test', 'test.rs', 'rust',
                         1, 1, 0, 0, 0, 'String', NULL, NULL, '[]', '[]', NULL)",
                [],
            )
            .unwrap();
    }

    #[test]
    fn test_migration_v2_idempotent() {
        let db = DatabaseConnection::open_in_memory().unwrap();

        // Run migration twice (idempotent)
        migrate_to_v2(db.conn()).unwrap();
        migrate_to_v2(db.conn()).unwrap();

        // Version is at latest since open_in_memory runs all migrations
        let version = get_schema_version(db.conn()).unwrap();
        assert!(version >= 2);
    }

    #[test]
    fn test_run_migrations() {
        let db = DatabaseConnection::open_in_memory().unwrap();

        // Run all migrations
        run_migrations(db.conn()).unwrap();

        // Should be at latest version
        let version = get_schema_version(db.conn()).unwrap();
        assert_eq!(version, 3);
    }

    #[test]
    fn test_enrichment_deps_table_created() {
        let db = DatabaseConnection::open_in_memory().unwrap();
        migrate_to_v2(db.conn()).unwrap();

        // Insert a test node first (required by foreign key)
        db.conn()
            .execute(
                "INSERT INTO nodes (id, kind, name, qualified_name, file_path, language,
                 start_line, end_line, start_column, end_column, updated_at)
                 VALUES ('node1', 'function', 'test', 'test', 'test.rs', 'rust',
                         1, 1, 0, 0, 0)",
                [],
            )
            .unwrap();

        // Should be able to insert into enrichment_deps
        db.conn()
            .execute(
                "INSERT INTO enrichment_deps (node_id, depends_on_file) VALUES ('node1', 'other.rs')",
                [],
            )
            .unwrap();

        // Should be able to query
        let count: i32 = db
            .conn()
            .query_row("SELECT COUNT(*) FROM enrichment_deps", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_metadata_table_created() {
        let db = DatabaseConnection::open_in_memory().unwrap();
        migrate_to_v2(db.conn()).unwrap();

        // Should be able to insert into metadata
        db.conn()
            .execute(
                "INSERT INTO metadata (key, value) VALUES ('lsp_enabled', 'true')",
                [],
            )
            .unwrap();

        // Should be able to query
        let value: String = db
            .conn()
            .query_row(
                "SELECT value FROM metadata WHERE key = 'lsp_enabled'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(value, "true");
    }

    #[test]
    fn test_migration_to_v3() {
        let db = DatabaseConnection::open_in_memory().unwrap();
        // V3 already applied by open_in_memory -> run_migrations
        migrate_to_v3(db.conn()).unwrap();
        let version = get_schema_version(db.conn()).unwrap();
        assert_eq!(version, 3);
    }

    #[test]
    fn test_migration_v3_idempotent() {
        let db = DatabaseConnection::open_in_memory().unwrap();
        migrate_to_v3(db.conn()).unwrap();
        migrate_to_v3(db.conn()).unwrap();
        assert_eq!(get_schema_version(db.conn()).unwrap(), 3);
    }

    #[test]
    fn test_edge_dedup_unique_index() {
        let db = DatabaseConnection::open_in_memory().unwrap();

        // Insert a test node first (edges reference nodes)
        db.conn()
            .execute(
                "INSERT INTO nodes (id, kind, name, qualified_name, file_path, language,
                 start_line, end_line, start_column, end_column, updated_at)
                 VALUES ('n1', 'function', 'f', 'f', 'a.rs', 'rust', 1, 1, 0, 0, 0)",
                [],
            )
            .unwrap();

        // First edge insert succeeds
        db.conn()
            .execute(
                "INSERT OR IGNORE INTO edges (source, target, kind, line, col) VALUES ('n1', 'n1', 'calls', 1, 0)",
                [],
            )
            .unwrap();

        // Duplicate insert is silently ignored
        db.conn()
            .execute(
                "INSERT OR IGNORE INTO edges (source, target, kind, line, col) VALUES ('n1', 'n1', 'calls', 1, 0)",
                [],
            )
            .unwrap();

        // Count should be 1
        let count: i64 = db
            .conn()
            .query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);

        // Same edge at different line is distinct
        db.conn()
            .execute(
                "INSERT OR IGNORE INTO edges (source, target, kind, line, col) VALUES ('n1', 'n1', 'calls', 5, 0)",
                [],
            )
            .unwrap();

        let count: i64 = db
            .conn()
            .query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 2);
    }
}
