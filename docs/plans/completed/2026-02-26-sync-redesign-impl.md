# Sync Redesign Implementation Plan

> **Status:** IMPLEMENTED (2026-02-28) — All milestones verified complete. Edge dedup, scoped resolution, ImpactCapture, locking, git diff, checkpoints, pending sync — all operational. All 8 MCP tools working with resolved edges.

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Redesign CodeGraph's sync pipeline from trigger to embedding update — covering git hook triggers, change detection, safe extraction, scoped resolution, and incremental embedding as one end-to-end system.

**Architecture:** Clean Architecture with 4 new focused modules (`pending.rs`, `git_diff.rs`, `checkpoint.rs`, `impact.rs`). `SyncManager` becomes a thin extraction executor. `CodeGraph::sync_with_options()` orchestrates the full 7-phase pipeline. Lock scope widens to cover detect→embed. `EdgeSnapshot` moves off hot path, replaced by targeted `ImpactCapture` queries.

**Tech Stack:** Rust, SQLite (rusqlite), tree-sitter, clap, sha2, filetime, serde_json

**Design doc:** `codegraph/docs/plans/2026-02-22-sync-redesign.md`

---

## Milestone 1: Standalone Safety Fixes

### Task 1: Add `clear_all_graph_data()` to QueryBuilder

**Files:**
- Modify: `crates/codegraph-db/src/queries.rs`

**Step 1: Add the method**

Add to `QueryBuilder` impl block (after `delete_metadata` around line 903):

```rust
/// Clear all graph data (nodes, edges, files, unresolved_refs).
/// Used by index_all() to do a clean rebuild.
pub fn clear_all_graph_data(&self, conn: &Connection) -> Result<(), DbError> {
    // Order matters: edges have FK to nodes, unresolved_refs have FK to nodes
    conn.execute_batch(
        "DELETE FROM edges;
         DELETE FROM unresolved_refs;
         DELETE FROM nodes;
         DELETE FROM files;"
    )?;
    Ok(())
}
```

**Step 2: Test**

Add test in the existing `#[cfg(test)] mod tests` block:

```rust
#[test]
fn test_clear_all_graph_data() {
    let db = DatabaseConnection::open_in_memory().unwrap();
    let queries = QueryBuilder::new(db.conn()).unwrap();

    // Insert a node
    let node = Node::new("n1", NodeKind::Function, "foo", "test::foo", "test.rs",
        Language::Rust, 1, 10);
    queries.insert_node(db.conn(), &node).unwrap();

    // Insert an edge
    let edge = Edge::new("n1", "n1", EdgeKind::Contains);
    queries.insert_edge(db.conn(), &edge).unwrap();

    // Verify data exists
    let stats = queries.get_stats(db.conn()).unwrap();
    assert!(stats.node_count > 0);

    // Clear
    queries.clear_all_graph_data(db.conn()).unwrap();

    // Verify empty
    let stats = queries.get_stats(db.conn()).unwrap();
    assert_eq!(stats.node_count, 0);
    assert_eq!(stats.edge_count, 0);
}
```

**Step 3: Run tests**

Run: `cd codegraph && cargo test -p codegraph-db -- test_clear_all_graph_data`
Expected: PASS

**Step 4: Commit**

```
feat(db): add clear_all_graph_data() for clean index rebuilds
```

---

### Task 2: Fix `index_all()` — add lock + full table clear

**Files:**
- Modify: `crates/codegraph-core/src/codegraph.rs`

**Step 1: Update index_all()**

Replace lines 126-153 (the beginning of `index_all`) with:

```rust
pub fn index_all(&mut self) -> Result<IndexingResult, CodeGraphError> {
    use codegraph_sync::IndexLock;
    use codegraph_types::FileRecord;
    use std::time::{SystemTime, UNIX_EPOCH};

    // Acquire lock to prevent concurrent sync during full reindex
    let _lock = if self.config.data_dir.exists() {
        match IndexLock::acquire(&self.config.data_dir) {
            Ok(lock) => Some(lock),
            Err(codegraph_sync::SyncError::LockHeld) => {
                return Err(CodeGraphError::Other(
                    "Cannot reindex: sync operation in progress".to_string(),
                ));
            }
            Err(e) => return Err(CodeGraphError::Other(e.to_string())),
        }
    } else {
        None
    };

    let extraction_config = Config {
        root_dir: self.config.root.to_string_lossy().to_string(),
        exclude: self.config.exclude_patterns.clone(),
        max_file_size: self.config.max_file_size as u64,
        ..Config::default()
    };

    let orchestrator = ExtractionOrchestrator::new(&self.config.root, extraction_config.clone())?;
    let mut nodes_created = 0;
    let mut edges_created = 0;
    let mut files_indexed = 0;

    let scan_result = codegraph_extraction::FileScanner::new(
        &self.config.root,
        &extraction_config,
    )?
    .scan()?;

    log::info!("Found {} files to index", scan_result.files.len());

    // Clear ALL graph data before rebuild (not just unresolved_refs)
    self.queries.clear_all_graph_data(self.db.conn())?;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    // ... rest of the for loop remains unchanged from line 160 onward ...
```

Note: The `clear_unresolved_refs` call at line 153 is now redundant (covered by `clear_all_graph_data`) — remove it.

**Step 2: Add `Other` variant to `CodeGraphError` if not present**

Check if `CodeGraphError` has an `Other(String)` variant. If not, add it.

**Step 3: Run tests**

Run: `cd codegraph && cargo test -p codegraph-core`
Expected: All existing tests pass. The `test_sync_returns_full_sync_result` test calls `index_all()` before sync — it should still work because `index_all()` now clears everything first.

**Step 4: Commit**

```
fix(core): index_all() clears all tables before rebuild and acquires lock
```

---

### Task 3: Fix `process_modify()` — parse before delete

**Files:**
- Modify: `crates/codegraph-sync/src/sync.rs`

**Step 1: Rewrite process_modify()**

Replace the current `process_modify` method (lines 360-402) with:

```rust
/// Process a modified file, returning (old_count, new_count, old_node_ids)
///
/// Safety: parses new content OUTSIDE any transaction. If extraction fails,
/// old nodes are preserved (no data loss).
fn process_modify(
    &self,
    conn: &Connection,
    queries: &mut QueryBuilder,
    change: &FileChange,
) -> Result<(usize, usize, Vec<String>), SyncError> {
    // 1. Parse new content OUTSIDE transaction — failure preserves old data
    let full_path = std::path::Path::new(&self.base_path).join(&change.path);
    let content = std::fs::read_to_string(&full_path)?;
    let result = self.registry.extract(&content, &change.path, change.language)?;

    // 2. Capture old node IDs before deletion
    let old_nodes = queries.get_nodes_by_file(conn, &change.path)?;
    let old_count = old_nodes.len();
    let old_node_ids: Vec<String> = old_nodes.iter().map(|n| n.id.0.clone()).collect();

    // 3. Atomic swap: delete old + insert new
    queries.delete_nodes_by_file(conn, &change.path)?;

    let new_count = result.nodes.len();
    queries.insert_nodes(conn, &result.nodes)?;
    queries.insert_edges(conn, &result.edges)?;

    for ref_info in &result.unresolved_references {
        queries.insert_unresolved_ref(conn, ref_info)?;
    }

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
```

The key change: `read_to_string` and `extract` now happen BEFORE `delete_nodes_by_file`. If either fails, the function returns `Err` and old nodes remain intact.

**Step 2: Run tests**

Run: `cd codegraph && cargo test -p codegraph-sync`
Expected: All existing tests pass (behavior unchanged for happy path).

**Step 3: Commit**

```
fix(sync): process_modify parses before deleting to prevent data loss
```

---

## Milestone 2: Resolution & Edge Diff Redesign

### Task 4: Schema migration v3 — edge dedup + resolved column

**Files:**
- Modify: `crates/codegraph-db/src/migrations.rs`

**Step 1: Add migrate_to_v3()**

```rust
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
```

**Step 2: Update run_migrations()**

```rust
pub fn run_migrations(conn: &Connection) -> Result<(), DbError> {
    let current = get_schema_version(conn)?;
    if current < 2 { migrate_to_v2(conn)?; }
    if current < 3 { migrate_to_v3(conn)?; }
    Ok(())
}
```

**Step 3: Add tests**

```rust
#[test]
fn test_migration_to_v3() {
    let db = DatabaseConnection::open_in_memory().unwrap();
    // V2 already applied by open_in_memory -> run_migrations
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

    // Insert a test node
    db.conn().execute(
        "INSERT INTO nodes (id, kind, name, qualified_name, file_path, language,
         start_line, end_line, start_column, end_column, updated_at)
         VALUES ('n1', 'function', 'f', 'f', 'a.rs', 'rust', 1, 1, 0, 0, 0)",
        [],
    ).unwrap();

    // First edge insert succeeds
    db.conn().execute(
        "INSERT OR IGNORE INTO edges (source, target, kind, line, col) VALUES ('n1', 'n1', 'calls', 1, 0)",
        [],
    ).unwrap();

    // Duplicate insert is silently ignored
    db.conn().execute(
        "INSERT OR IGNORE INTO edges (source, target, kind, line, col) VALUES ('n1', 'n1', 'calls', 1, 0)",
        [],
    ).unwrap();

    // Count should be 1
    let count: i64 = db.conn().query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0)).unwrap();
    assert_eq!(count, 1);

    // Same edge at different line is distinct
    db.conn().execute(
        "INSERT OR IGNORE INTO edges (source, target, kind, line, col) VALUES ('n1', 'n1', 'calls', 5, 0)",
        [],
    ).unwrap();

    let count: i64 = db.conn().query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0)).unwrap();
    assert_eq!(count, 2);
}
```

**Step 4: Run tests**

Run: `cd codegraph && cargo test -p codegraph-db`
Expected: PASS

**Step 5: Commit**

```
feat(db): add v3 migration with edge dedup index and resolved refs column
```

---

### Task 5: Update edge insertion to INSERT OR IGNORE + new query methods

**Files:**
- Modify: `crates/codegraph-db/src/queries.rs`

**Step 1: Change insert_edge to INSERT OR IGNORE**

Replace line 498-501:
```rust
// Before: INSERT INTO edges ...
// After:
"INSERT OR IGNORE INTO edges (source, target, kind, metadata, line, col)
 VALUES (?1, ?2, ?3, ?4, ?5, ?6)"
```

**Step 2: Add scoped query methods**

Add these methods to `QueryBuilder`:

```rust
/// Get unresolved refs from nodes in the given files (source-scoped resolution).
/// Only returns refs where resolved = 0.
pub fn get_unresolved_refs_by_files(
    &self,
    conn: &Connection,
    file_paths: &[&str],
) -> Result<Vec<UnresolvedReference>, DbError> {
    if file_paths.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders: Vec<String> = (1..=file_paths.len()).map(|i| format!("?{}", i)).collect();
    let sql = format!(
        r#"SELECT ur.* FROM unresolved_refs ur
           JOIN nodes n ON ur.from_node_id = n.id
           WHERE n.file_path IN ({}) AND ur.resolved = 0"#,
        placeholders.join(", ")
    );
    let mut stmt = conn.prepare(&sql)?;
    let params: Vec<&dyn rusqlite::ToSql> = file_paths.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
    let refs = stmt.query_map(params.as_slice(), Self::row_to_unresolved_ref)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(refs)
}

/// Get symbol names defined in the given files (for target-scoped resolution).
pub fn get_symbol_names_in_files(
    &self,
    conn: &Connection,
    file_paths: &[&str],
) -> Result<Vec<String>, DbError> {
    if file_paths.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders: Vec<String> = (1..=file_paths.len()).map(|i| format!("?{}", i)).collect();
    let sql = format!(
        "SELECT DISTINCT name FROM nodes WHERE file_path IN ({})",
        placeholders.join(", ")
    );
    let mut stmt = conn.prepare(&sql)?;
    let params: Vec<&dyn rusqlite::ToSql> = file_paths.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
    let names = stmt.query_map(params.as_slice(), |row| row.get(0))?
        .collect::<Result<Vec<String>, _>>()?;
    Ok(names)
}

/// Get unresolved refs matching given names, with frequency cap.
/// Skips names with > cap matching unresolved refs.
pub fn get_unresolved_refs_by_names_capped(
    &self,
    conn: &Connection,
    names: &[&str],
    cap: usize,
) -> Result<Vec<UnresolvedReference>, DbError> {
    let mut result = Vec::new();
    for name in names {
        let count: usize = conn.query_row(
            "SELECT COUNT(*) FROM unresolved_refs WHERE reference_name = ?1 AND resolved = 0",
            params![name],
            |row| row.get(0),
        )?;
        if count > cap {
            log::debug!("Skipping target-scoped resolution for '{}': {} refs > cap {}", name, count, cap);
            continue;
        }
        let mut stmt = conn.prepare(
            "SELECT * FROM unresolved_refs WHERE reference_name = ?1 AND resolved = 0"
        )?;
        let refs = stmt.query_map(params![name], Self::row_to_unresolved_ref)?
            .collect::<Result<Vec<_>, _>>()?;
        result.extend(refs);
    }
    Ok(result)
}

/// Mark an unresolved ref as resolved (set resolved = 1 instead of deleting).
pub fn mark_unresolved_ref_resolved(
    &self,
    conn: &Connection,
    from_node_id: &str,
    reference_name: &str,
) -> Result<(), DbError> {
    conn.execute(
        "UPDATE unresolved_refs SET resolved = 1 WHERE from_node_id = ?1 AND reference_name = ?2 AND resolved = 0",
        params![from_node_id, reference_name],
    )?;
    Ok(())
}
```

**Step 3: Update get_all_unresolved_refs to filter resolved**

```rust
pub fn get_all_unresolved_refs(&self, conn: &Connection) -> Result<Vec<UnresolvedReference>, DbError> {
    let mut stmt = conn.prepare("SELECT * FROM unresolved_refs WHERE resolved = 0")?;
    let refs = stmt
        .query_map([], Self::row_to_unresolved_ref)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(refs)
}
```

**Step 4: Update row_to_unresolved_ref to handle optional `resolved` column**

Check if `row_to_unresolved_ref` needs updating — the `resolved` column is new and won't be in the `SELECT *` result mapping to `UnresolvedReference`. Since `UnresolvedReference` doesn't have a `resolved` field (it's a DB-only column), no struct change needed — `SELECT *` will just include the extra column which rusqlite ignores in positional mapping. Verify this works.

**Step 5: Run tests**

Run: `cd codegraph && cargo test -p codegraph-db`
Expected: PASS

**Step 6: Commit**

```
feat(db): INSERT OR IGNORE for edges, add scoped resolution queries
```

---

### Task 6: Create `impact.rs` — pre-delete impact capture

**Files:**
- Create: `crates/codegraph-sync/src/impact.rs`
- Modify: `crates/codegraph-sync/src/lib.rs`

**Step 1: Create impact.rs**

```rust
//! Pre-delete impact capture for embedding candidate computation.
//!
//! Captures IDs of nodes affected by deleting/modifying a set of nodes,
//! BEFORE those nodes are deleted (while edges still exist in the DB).
//! Replaces EdgeSnapshot on the hot path with targeted queries proportional
//! to changed files, not all edges.

use codegraph_db::rusqlite::{params, Connection};
use codegraph_db::DbError;
use std::collections::HashSet;

/// Maximum number of IDs per SQL IN clause chunk.
const CHUNK_SIZE: usize = 500;

/// Result of pre-delete impact capture.
#[derive(Debug, Default)]
pub struct ImpactCapture {
    /// Node IDs that had edges into/from the about-to-be-deleted nodes.
    pub affected_ids: HashSet<String>,
    /// Sibling node IDs (share Contains parent with changed nodes).
    pub sibling_ids: HashSet<String>,
}

impl ImpactCapture {
    /// Create an empty impact capture.
    pub fn new() -> Self {
        Self::default()
    }

    /// Query pre-delete impact for a batch of node IDs.
    /// Must be called BEFORE deleting the nodes (edges must still exist).
    /// Chunked at 500 IDs per query.
    pub fn capture(conn: &Connection, old_node_ids: &[&str]) -> Result<Self, DbError> {
        if old_node_ids.is_empty() {
            return Ok(Self::new());
        }

        let mut result = Self::new();

        for chunk in old_node_ids.chunks(CHUNK_SIZE) {
            let neighbors = query_edge_neighbors(conn, chunk)?;
            result.affected_ids.extend(neighbors);

            let siblings = query_contains_siblings(conn, chunk)?;
            result.sibling_ids.extend(siblings);
        }

        // Remove the input IDs themselves from results
        // (they're the ones being deleted, not "affected")
        for id in old_node_ids {
            result.affected_ids.remove(*id);
            result.sibling_ids.remove(*id);
        }

        Ok(result)
    }

    /// Merge another ImpactCapture into this one.
    pub fn merge(&mut self, other: ImpactCapture) {
        self.affected_ids.extend(other.affected_ids);
        self.sibling_ids.extend(other.sibling_ids);
    }
}

/// Query nodes connected via calls/extends/implements/references edges.
fn query_edge_neighbors(conn: &Connection, ids: &[&str]) -> Result<HashSet<String>, DbError> {
    let placeholders: Vec<String> = (1..=ids.len()).map(|i| format!("?{}", i)).collect();
    let ph = placeholders.join(",");

    let sql = format!(
        r#"SELECT DISTINCT e.source AS affected_id FROM edges e
           WHERE e.target IN ({ph}) AND e.kind IN ('calls','extends','implements','references')
           UNION
           SELECT DISTINCT e.target AS affected_id FROM edges e
           WHERE e.source IN ({ph}) AND e.kind IN ('calls','extends','implements','references')"#
    );

    let mut stmt = conn.prepare(&sql)?;
    // Build params: need ids twice (once for each IN clause)
    let mut param_values: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    for id in ids {
        param_values.push(Box::new(id.to_string()));
    }
    // Duplicate for second IN clause
    let double_params: Vec<Box<dyn rusqlite::ToSql>> = ids.iter()
        .chain(ids.iter())
        .map(|id| Box::new(id.to_string()) as Box<dyn rusqlite::ToSql>)
        .collect();
    let param_refs: Vec<&dyn rusqlite::ToSql> = double_params.iter().map(|p| p.as_ref()).collect();

    let results = stmt.query_map(param_refs.as_slice(), |row| row.get::<_, String>(0))?
        .collect::<Result<HashSet<String>, _>>()?;

    Ok(results)
}

/// Query siblings: nodes that share a Contains parent with the given nodes.
fn query_contains_siblings(conn: &Connection, ids: &[&str]) -> Result<HashSet<String>, DbError> {
    let placeholders: Vec<String> = (1..=ids.len()).map(|i| format!("?{}", i)).collect();
    let ph = placeholders.join(",");

    let sql = format!(
        r#"SELECT DISTINCT e2.target AS sibling_id
           FROM edges e1
           JOIN edges e2 ON e1.source = e2.source AND e2.kind = 'contains'
           WHERE e1.target IN ({ph}) AND e1.kind = 'contains'
             AND e2.target NOT IN ({ph})"#
    );

    let mut stmt = conn.prepare(&sql)?;
    let double_params: Vec<Box<dyn rusqlite::ToSql>> = ids.iter()
        .chain(ids.iter())
        .map(|id| Box::new(id.to_string()) as Box<dyn rusqlite::ToSql>)
        .collect();
    let param_refs: Vec<&dyn rusqlite::ToSql> = double_params.iter().map(|p| p.as_ref()).collect();

    let results = stmt.query_map(param_refs.as_slice(), |row| row.get::<_, String>(0))?
        .collect::<Result<HashSet<String>, _>>()?;

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use codegraph_db::DatabaseConnection;

    fn setup_test_db() -> DatabaseConnection {
        let db = DatabaseConnection::open_in_memory().unwrap();
        // Insert test nodes
        for (id, file) in [("a", "a.rs"), ("b", "a.rs"), ("c", "b.rs"), ("parent", "a.rs")] {
            db.conn().execute(
                "INSERT INTO nodes (id, kind, name, qualified_name, file_path, language,
                 start_line, end_line, start_column, end_column, updated_at)
                 VALUES (?1, 'function', ?1, ?1, ?2, 'rust', 1, 1, 0, 0, 0)",
                params![id, file],
            ).unwrap();
        }
        db
    }

    #[test]
    fn test_impact_capture_empty() {
        let db = setup_test_db();
        let impact = ImpactCapture::capture(db.conn(), &[]).unwrap();
        assert!(impact.affected_ids.is_empty());
        assert!(impact.sibling_ids.is_empty());
    }

    #[test]
    fn test_impact_capture_edge_neighbors() {
        let db = setup_test_db();
        // c calls a
        db.conn().execute(
            "INSERT INTO edges (source, target, kind) VALUES ('c', 'a', 'calls')", []
        ).unwrap();

        // Capture impact of deleting 'a'
        let impact = ImpactCapture::capture(db.conn(), &["a"]).unwrap();
        assert!(impact.affected_ids.contains("c"));
        assert!(!impact.affected_ids.contains("a")); // self excluded
    }

    #[test]
    fn test_impact_capture_siblings() {
        let db = setup_test_db();
        // parent contains a and b
        db.conn().execute(
            "INSERT INTO edges (source, target, kind) VALUES ('parent', 'a', 'contains')", []
        ).unwrap();
        db.conn().execute(
            "INSERT INTO edges (source, target, kind) VALUES ('parent', 'b', 'contains')", []
        ).unwrap();

        let impact = ImpactCapture::capture(db.conn(), &["a"]).unwrap();
        assert!(impact.sibling_ids.contains("b"));
        assert!(!impact.sibling_ids.contains("a")); // self excluded
    }
}
```

**Step 2: Add to lib.rs**

Add `pub mod impact;` and `pub use impact::ImpactCapture;` to `crates/codegraph-sync/src/lib.rs`.

**Step 3: Run tests**

Run: `cd codegraph && cargo test -p codegraph-sync -- impact`
Expected: PASS

**Step 4: Commit**

```
feat(sync): add ImpactCapture for pre-delete neighbor detection
```

---

### Task 7: Integrate ImpactCapture into SyncManager + add to SyncResult

**Files:**
- Modify: `crates/codegraph-sync/src/sync.rs`

**Step 1: Add ImpactCapture to SyncResult**

```rust
use crate::impact::ImpactCapture;

pub struct SyncResult {
    pub stats: SyncStats,
    pub had_changes: bool,
    pub duration_ms: u64,
    pub enrichment_scope: SelectiveScope,
    pub deleted_node_ids: Vec<String>,
    pub changed_file_paths: Vec<String>,
    /// Pre-delete impact: neighbors and siblings of modified/deleted nodes.
    pub pre_delete_impact: ImpactCapture,
}
```

**Step 2: Update process_modify to capture impact before delete**

Modify `process_modify` to call `ImpactCapture::capture()` AFTER getting old node IDs but BEFORE deleting:

```rust
fn process_modify(...) -> Result<(usize, usize, Vec<String>, ImpactCapture), SyncError> {
    let full_path = std::path::Path::new(&self.base_path).join(&change.path);
    let content = std::fs::read_to_string(&full_path)?;
    let result = self.registry.extract(&content, &change.path, change.language)?;

    let old_nodes = queries.get_nodes_by_file(conn, &change.path)?;
    let old_count = old_nodes.len();
    let old_node_ids: Vec<String> = old_nodes.iter().map(|n| n.id.0.clone()).collect();
    let old_id_refs: Vec<&str> = old_node_ids.iter().map(|s| s.as_str()).collect();

    // Capture impact BEFORE deletion (edges still exist)
    let impact = ImpactCapture::capture(conn, &old_id_refs)?;

    // Now safe to delete and re-insert
    queries.delete_nodes_by_file(conn, &change.path)?;
    // ... insert new nodes/edges/refs/file ...

    Ok((old_count, new_count, old_node_ids, impact))
}
```

**Step 3: Update process_delete similarly**

```rust
fn process_delete(...) -> Result<(usize, Vec<String>, ImpactCapture), SyncError> {
    let old_nodes = queries.get_nodes_by_file(conn, &change.path)?;
    let old_node_ids: Vec<String> = old_nodes.iter().map(|n| n.id.0.clone()).collect();
    let old_id_refs: Vec<&str> = old_node_ids.iter().map(|s| s.as_str()).collect();
    let impact = ImpactCapture::capture(conn, &old_id_refs)?;
    queries.delete_file(conn, &change.path)?;
    Ok((old_nodes.len(), old_node_ids, impact))
}
```

**Step 4: Update process_changes to accumulate ImpactCapture**

Accumulate impacts across all file changes and return in SyncResult:

```rust
fn process_changes(...) -> Result<(Vec<String>, ImpactCapture), SyncError> {
    let mut deleted_node_ids = Vec::new();
    let mut accumulated_impact = ImpactCapture::new();

    for change in changes {
        match change.kind {
            ChangeKind::Modified => {
                let (deleted, added, old_ids, impact) = self.process_modify(...)?;
                deleted_node_ids.extend(old_ids);
                accumulated_impact.merge(impact);
            }
            ChangeKind::Deleted => {
                let (count, old_ids, impact) = self.process_delete(...)?;
                deleted_node_ids.extend(old_ids);
                accumulated_impact.merge(impact);
            }
            // Added files don't need impact capture
            ChangeKind::Added => { ... }
        }
    }
    Ok((deleted_node_ids, accumulated_impact))
}
```

**Step 5: Update sync_with_codegraph_dir to return impact in SyncResult**

Wire `pre_delete_impact` into the returned `SyncResult`.

**Step 6: Run tests**

Run: `cd codegraph && cargo test -p codegraph-sync`
Expected: PASS

**Step 7: Commit**

```
feat(sync): integrate ImpactCapture into sync pipeline
```

---

### Task 8: Add scoped resolution — `resolve_for_files()`

**Files:**
- Modify: `crates/codegraph-resolution/src/resolver.rs`

**Step 1: Add ScopedResolutionResult struct**

```rust
/// Result of scoped resolution, carrying affected node IDs for embed candidate computation.
#[derive(Debug, Default)]
pub struct ScopedResolutionResult {
    pub stats: ResolutionStats,
    /// Source node IDs of newly created edges.
    pub source_node_ids: HashSet<String>,
    /// Target node IDs of newly created edges.
    pub target_node_ids: HashSet<String>,
}
```

Add `use std::collections::HashSet;` at the top.

**Step 2: Add resolve_for_files() method**

```rust
/// Resolve references scoped to changed files only.
///
/// Source-scoped: refs FROM nodes in changed_files.
/// Target-scoped: refs TO symbols whose names match nodes in changed_files.
pub fn resolve_for_files(
    &mut self,
    changed_files: &[&str],
) -> Result<ScopedResolutionResult, ResolutionError> {
    let mut result = ScopedResolutionResult::default();

    // Source-scoped: resolve unresolved refs from changed files
    let source_refs = self.queries.get_unresolved_refs_by_files(self.conn, changed_files)?;
    log::info!("Source-scoped resolution: {} refs from changed files", source_refs.len());

    for unresolved_ref in source_refs {
        let res = self.resolve_reference_with_kind(
            unresolved_ref.from_node_id.as_str(),
            &unresolved_ref.reference_name,
            unresolved_ref.reference_kind,
        )?;
        result.stats.total_processed += 1;
        if let Some(ref target) = res.target {
            result.stats.resolved += 1;
            result.source_node_ids.insert(unresolved_ref.from_node_id.clone());
            result.target_node_ids.insert(target.node_id.as_str().to_string());
        } else {
            result.stats.unresolved += 1;
        }
    }

    // Target-scoped: find refs TO symbols in changed files
    let symbol_names = self.queries.get_symbol_names_in_files(self.conn, changed_files)?;
    let name_refs: Vec<&str> = symbol_names.iter().map(|s| s.as_str()).collect();
    let target_refs = self.queries.get_unresolved_refs_by_names_capped(self.conn, &name_refs, 100)?;
    log::info!("Target-scoped resolution: {} refs to symbols in changed files", target_refs.len());

    for unresolved_ref in target_refs {
        let res = self.resolve_reference_with_kind(
            unresolved_ref.from_node_id.as_str(),
            &unresolved_ref.reference_name,
            unresolved_ref.reference_kind,
        )?;
        result.stats.total_processed += 1;
        if let Some(ref target) = res.target {
            result.stats.resolved += 1;
            result.source_node_ids.insert(unresolved_ref.from_node_id.clone());
            result.target_node_ids.insert(target.node_id.as_str().to_string());
        } else {
            result.stats.unresolved += 1;
        }
    }

    Ok(result)
}
```

**Step 3: Change resolve_reference_with_kind to use mark_resolved**

Replace line 259-260:
```rust
// Before: self.queries.delete_unresolved_reference(...)
// After:
self.queries.mark_unresolved_ref_resolved(self.conn, source_id, ref_name)?;
```

**Step 4: Run tests**

Run: `cd codegraph && cargo test -p codegraph-resolution`
Expected: PASS (existing tests should still work since mark_resolved has same visible effect — ref no longer returned by get_all_unresolved_refs)

**Step 5: Commit**

```
feat(resolution): add resolve_for_files() with two-sided scoped resolution
```

---

### Task 9: Wire new sync flow in CodeGraph::sync()

**Files:**
- Modify: `crates/codegraph-core/src/codegraph.rs`

**Step 1: Update sync() to use ImpactCapture + scoped resolution**

Replace the current `sync()` method. Key changes:
- Use `ImpactCapture` from `sync_result.pre_delete_impact` instead of `EdgeSnapshot`
- Call `resolve_for_files()` instead of `resolve_all()`
- Compute embed candidates from impact + resolution result
- Add full-reindex fallback check (>30% threshold)

The `EdgeSnapshot::capture()` calls (pre and post) are removed from the hot path. The incremental `sync_embeddings()` method signature changes to accept `ImpactCapture` + `ScopedResolutionResult` instead of `EdgeDiff`.

This is a larger refactor — see the design doc Phase 5 for the exact embed candidate formula:
```
candidates = changed_nodes ∪ pre_delete_affected ∪ resolver_source ∪ resolver_target ∪ siblings - deleted
```

**Step 2: Run all tests**

Run: `cd codegraph && cargo test`
Expected: PASS

**Step 3: Commit**

```
feat(core): wire ImpactCapture + scoped resolution into sync pipeline
```

---

## Milestone 3: Hook & Lock Operability

### Task 10: Create `checkpoint.rs`

**Files:**
- Create: `crates/codegraph-sync/src/checkpoint.rs`

Small focused module for `sync.last_head` management using the metadata table.

```rust
//! Checkpoint management for sync.last_head.

use codegraph_db::{QueryBuilder, DbError};
use rusqlite::Connection;
use std::path::Path;
use std::process::Command;

pub const LAST_HEAD_KEY: &str = "sync.last_head";
pub const LAST_TIMESTAMP_KEY: &str = "sync.last_timestamp";

pub fn read_last_head(conn: &Connection, queries: &QueryBuilder) -> Result<Option<String>, DbError> {
    queries.get_metadata(conn, LAST_HEAD_KEY)
}

pub fn write_last_head(conn: &Connection, queries: &QueryBuilder, head: &str) -> Result<(), DbError> {
    queries.set_metadata(conn, LAST_HEAD_KEY, head)
}

pub fn write_last_timestamp(conn: &Connection, queries: &QueryBuilder) -> Result<(), DbError> {
    let now = chrono::Utc::now().to_rfc3339();
    queries.set_metadata(conn, LAST_TIMESTAMP_KEY, &now)
}

/// Get current git HEAD SHA. Returns None if not in a git repo.
pub fn get_git_head(repo_root: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo_root)
        .output()
        .ok()?;
    if !output.status.success() { return None; }
    let head = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if head.is_empty() { None } else { Some(head) }
}
```

Note: Check if `chrono` is already a dependency. If not, use `SystemTime` instead:
```rust
use std::time::{SystemTime, UNIX_EPOCH};
let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs().to_string();
```

**Commit:** `feat(sync): add checkpoint module for sync.last_head management`

---

### Task 11: Create `pending.rs`

**Files:**
- Create: `crates/codegraph-sync/src/pending.rs`

Manages `sync.pending` / `sync.processing` lifecycle.

```rust
//! Pending sync coalescing.
//!
//! When a git hook fires during an active sync, instead of losing the event,
//! write sync.pending. The running sync drains pending on completion.

use crate::error::SyncError;
use std::fs;
use std::path::{Path, PathBuf};

const ALLOWED_HOOKS: &[&str] = &["post-commit", "post-checkout", "post-merge", "post-rewrite"];

pub struct PendingSync {
    codegraph_dir: PathBuf,
}

impl PendingSync {
    pub fn new(codegraph_dir: &Path) -> Self {
        Self { codegraph_dir: codegraph_dir.to_path_buf() }
    }

    fn pending_path(&self) -> PathBuf { self.codegraph_dir.join("sync.pending") }
    fn processing_path(&self) -> PathBuf { self.codegraph_dir.join("sync.processing") }

    /// Write sync.pending atomically (write temp, rename).
    pub fn write(&self, hook_name: &str) -> Result<(), SyncError> {
        if !ALLOWED_HOOKS.contains(&hook_name) {
            return Err(SyncError::Other(format!("Invalid hook name: {}", hook_name)));
        }
        let payload = format!(
            r#"{{"hook":"{}","timestamp":"{}"}}"#,
            hook_name,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
        );
        let temp_path = self.codegraph_dir.join("sync.pending.tmp");
        fs::write(&temp_path, &payload)?;
        fs::rename(&temp_path, self.pending_path())?;
        log::info!("Wrote sync.pending for hook {}", hook_name);
        Ok(())
    }

    /// Atomically claim pending by rename to processing.
    /// Returns Some(hook_name) if claimed, None if no pending file.
    pub fn claim(&self) -> Result<Option<String>, SyncError> {
        let pending = self.pending_path();
        if !pending.exists() { return Ok(None); }

        let processing = self.processing_path();
        fs::rename(&pending, &processing)?;

        let content = fs::read_to_string(&processing)?;
        // Parse hook name from JSON payload
        let hook = content.split('"').nth(3).unwrap_or("unknown").to_string();
        Ok(Some(hook))
    }

    /// Delete sync.processing after drain completes.
    pub fn complete_processing(&self) -> Result<(), SyncError> {
        let processing = self.processing_path();
        if processing.exists() { fs::remove_file(&processing)?; }
        Ok(())
    }

    /// Check if sync.pending exists.
    pub fn has_pending(&self) -> bool {
        self.pending_path().exists()
    }

    /// Clean up pending/processing files (for uninstall).
    pub fn cleanup(&self) -> Result<(), SyncError> {
        for path in [self.pending_path(), self.processing_path()] {
            if path.exists() { fs::remove_file(&path)?; }
        }
        Ok(())
    }
}
```

**Commit:** `feat(sync): add PendingSync for hook event coalescing`

---

### Task 12: Create `git_diff.rs` — git diff-based change detection

**Files:**
- Create: `crates/codegraph-sync/src/git_diff.rs`

This is the most complex new module. It implements Phase 1 git-diff mode.

Key functionality:
- Read `sync.last_head` checkpoint from metadata
- Run `git diff --name-status -z <checkpoint>..HEAD` for committed changes
- Run `git diff --name-status -z HEAD` for working tree changes
- Run `git ls-files --others --exclude-standard -z` for untracked files
- Parse `--name-status -z` output (NUL-delimited, handle R/C renames)
- Classify candidates against files table
- Filter to supported languages + exclude patterns
- Fall back to hash scan on any git error

This file will be ~300 lines. See design doc Phase 1 for the complete specification.

**Commit:** `feat(sync): add GitDiffDetector for hook-triggered change detection`

---

### Task 13: Update `lock.rs` — add `try_acquire_or_pending()`

**Files:**
- Modify: `crates/codegraph-sync/src/lock.rs`

Add method that combines lock acquisition with pending-write on collision:

```rust
/// Attempt to acquire lock. On LockHeld in hook mode, write sync.pending.
/// Returns Ok(Some(lock)) on success, Ok(None) if pending was written.
pub fn try_acquire_or_pending(
    codegraph_dir: &Path,
    hook_name: &str,
) -> Result<Option<Self>, SyncError> {
    match Self::acquire(codegraph_dir) {
        Ok(lock) => Ok(Some(lock)),
        Err(SyncError::LockHeld) => {
            use crate::pending::PendingSync;
            PendingSync::new(codegraph_dir).write(hook_name)?;
            Ok(None)
        }
        Err(e) => Err(e),
    }
}
```

**Commit:** `feat(sync): add try_acquire_or_pending() to IndexLock`

---

### Task 14: Rewrite `git_hooks.rs`

**Files:**
- Modify: `crates/codegraph-sync/src/git_hooks.rs`

Major changes:
1. Add `post-rewrite` hook
2. New hook script template (from design spec — identical for all hooks)
3. Rename backup suffix from `.codegraph-backup` to `.codegraph-orig`
4. Use `git rev-parse --git-path hooks/<name>` instead of hardcoded `.git/hooks`
5. Add Husky/Lefthook detection with `--force` bypass
6. Add `.gitignore` management
7. Conflict detection: refuse if `.codegraph-orig` already exists

See the design doc Phase 0 for exact hook script. The `GitHooksManager` struct gets a `force: bool` field and `repo_root: PathBuf`.

**Commit:** `feat(sync): rewrite git hooks with new script, post-rewrite, and hook manager detection`

---

### Task 15: Update error types + lib.rs exports

**Files:**
- Modify: `crates/codegraph-sync/src/error.rs`
- Modify: `crates/codegraph-sync/src/lib.rs`

Add error variants: `PendingWriteFailed(String)`, `GitDiffFailed(String)`, `HookManagerDetected { tool: String }`.

Add module declarations and re-exports for `checkpoint`, `pending`, `git_diff`, `impact`.

**Commit:** `feat(sync): add new module exports and error variants`

---

### Task 16: Implement `SyncOptions` + `sync_with_options()` in CodeGraph

**Files:**
- Modify: `crates/codegraph-core/src/codegraph.rs`

This is the largest single task. Implement the full Phase 0-7 pipeline:

```rust
pub struct SyncOptions {
    pub hook_name: Option<String>,
    pub file_list: Option<Vec<String>>,
    pub verify_sync: bool,
}

pub fn sync_with_options(&mut self, opts: SyncOptions) -> Result<FullSyncResult, CodeGraphError> {
    // Phase 0/1: Detect changes (git-diff or hash-scan based on opts)
    // Phase 2: Acquire lock (or write pending + return early for hooks)
    // Revalidate changes under lock
    // Check full-reindex fallback (>30%)
    // Phase 3: SyncManager::process_changes (with ImpactCapture)
    // Phase 4: resolve_for_files() or resolve_all()
    // Phase 5: Compute embed candidates
    // Phase 6: Embed + cleanup + checkpoint + lock heartbeat
    // Phase 7: Release lock + drain pending loop
}
```

The existing `sync()` becomes `self.sync_with_options(SyncOptions::default())`.

**Commit:** `feat(core): implement full Phase 0-7 sync pipeline with SyncOptions`

---

### Task 17: Update CLI — add `--hook` and `--verify-sync` flags

**Files:**
- Modify: `crates/codegraph-cli/src/main.rs`
- Modify: `crates/codegraph-cli/src/commands.rs`

Add `--hook` and `--verify-sync` to the `Sync` subcommand. Update `commands::sync()` to forward options. Add hook-mode error handling (write `sync.failed`, exit 0). Update `status()` to surface `sync.failed`.

**Commit:** `feat(cli): add --hook and --verify-sync flags, sync.failed visibility`

---

### Task 18: Update MCP `are_hooks_installed()` — add post-rewrite

**Files:**
- Modify: `crates/codegraph-mcp/src/git.rs`

Add `"post-rewrite"` to the hook check array. Add `get_hooks_dir()` helper using `git rev-parse --git-path hooks`.

**Commit:** `feat(mcp): add post-rewrite to hook detection`

---

### Task 19: Add `.codegraph/` to `.gitignore` on init

**Files:**
- Modify: `crates/codegraph-core/src/codegraph.rs`

In `CodeGraph::init()`, after creating the `.codegraph` directory, call `ensure_codegraph_in_gitignore()` if a `.git` directory exists.

**Commit:** `feat(core): auto-add .codegraph/ to .gitignore on init`

---

## Milestone 4: Verification & Polish

### Task 20: Add `--verify-sync` EdgeSnapshot verification mode

**Files:**
- Modify: `crates/codegraph-core/src/codegraph.rs`

In `sync_with_options()`, when `opts.verify_sync` is true:
1. Capture pre-sync `EdgeSnapshot` (full)
2. Run normal sync (Phases 1-7)
3. Capture post-sync `EdgeSnapshot` (full)
4. `EdgeDiff::compute(before, after)`
5. Compare affected_nodes against Phase 5 candidates
6. Log any discrepancies

**Commit:** `feat(core): add --verify-sync EdgeSnapshot verification mode`

---

### Task 21: Build and test full workspace

**Step 1:** Run `cd codegraph && cargo build`
Expected: Clean compilation

**Step 2:** Run `cd codegraph && cargo test`
Expected: All tests pass

**Step 3:** Run `cd codegraph && cargo clippy`
Expected: No warnings

**Commit:** `chore: fix any clippy warnings from sync redesign`

---

## Dependency Graph

```
M1.1 (clear_all_graph_data) ─┬─> M1.2 (index_all fix)
                              └─> M2.4 (used by full-reindex fallback)
M1.3 (process_modify fix) ───> M2.4 (process_modify with impact)

M2.1 (migration v3) ─────────> M2.2 (INSERT OR IGNORE + queries)
M2.2 ─────────────────────────> M2.5 (resolve_for_files uses new queries)
M2.3 (impact.rs) ────────────> M2.4 (SyncManager integration)
M2.4 + M2.5 ─────────────────> M2.6 (wire into CodeGraph::sync)

M3.1 (checkpoint) ───┐
M3.2 (pending) ──────┤
M3.3 (lock update) ──┤
M3.4 (git_diff) ─────┤────────> M3.6 (sync_with_options)
M3.5 (git_hooks) ────┘
M3.6 ─────────────────────────> M3.7 + M3.8 (CLI updates)
M3.9 (MCP) ──────────────────> independent
M3.10 (gitignore) ───────────> depends on M3.5

M4.1 (verify mode) ──────────> depends on M3.6
```

Parallelizable within each milestone:
- M2: Tasks 4-6 can be done in parallel (migration, impact, queries)
- M3: Tasks 10-14 can be done in parallel (checkpoint, pending, git_diff, lock, hooks)
