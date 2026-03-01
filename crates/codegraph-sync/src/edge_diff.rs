//! Edge change detection (I11)
//!
//! Detects changes to graph edges between extraction passes to determine
//! which nodes need re-embedding due to structural changes.

use codegraph_db::rusqlite::Connection;
use std::collections::{HashMap, HashSet};

/// Represents a single edge in the graph
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct EdgeKey {
    pub source: String,
    pub target: String,
    pub kind: String,
}

impl EdgeKey {
    pub fn new(source: impl Into<String>, target: impl Into<String>, kind: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            target: target.into(),
            kind: kind.into(),
        }
    }
}

/// Snapshot of all edges in the graph at a point in time
#[derive(Debug, Default)]
pub struct EdgeSnapshot {
    /// All edges indexed by their key
    edges: HashSet<EdgeKey>,
    /// Edges grouped by source node
    by_source: HashMap<String, Vec<EdgeKey>>,
    /// Edges grouped by target node
    by_target: HashMap<String, Vec<EdgeKey>>,
}

impl EdgeSnapshot {
    /// Create an empty snapshot
    pub fn new() -> Self {
        Self::default()
    }

    /// Capture current edges from the database
    pub fn capture(conn: &Connection) -> Self {
        let mut snapshot = Self::new();

        let result = conn.prepare("SELECT source, target, kind FROM edges");
        if let Ok(mut stmt) = result {
            let rows = stmt.query_map([], |row| {
                Ok(EdgeKey {
                    source: row.get(0)?,
                    target: row.get(1)?,
                    kind: row.get(2)?,
                })
            });

            if let Ok(rows) = rows {
                for edge in rows.flatten() {
                    snapshot.add_edge(edge);
                }
            }
        }

        snapshot
    }

    /// Capture edges only for specific files
    pub fn capture_for_files(conn: &Connection, file_paths: &[&str]) -> Self {
        if file_paths.is_empty() {
            return Self::new();
        }

        let mut snapshot = Self::new();

        // Build placeholders for IN clause
        let placeholders: Vec<String> = (1..=file_paths.len()).map(|i| format!("?{}", i)).collect();
        let sql = format!(
            r#"SELECT DISTINCT e.source, e.target, e.kind
               FROM edges e
               JOIN nodes n ON (e.source = n.id OR e.target = n.id)
               WHERE n.file_path IN ({})"#,
            placeholders.join(", ")
        );

        if let Ok(mut stmt) = conn.prepare(&sql) {
            let params: Vec<&dyn rusqlite::ToSql> = file_paths
                .iter()
                .map(|s| s as &dyn rusqlite::ToSql)
                .collect();

            if let Ok(rows) = stmt.query_map(params.as_slice(), |row| {
                Ok(EdgeKey {
                    source: row.get(0)?,
                    target: row.get(1)?,
                    kind: row.get(2)?,
                })
            }) {
                for edge in rows.flatten() {
                    snapshot.add_edge(edge);
                }
            }
        }

        snapshot
    }

    /// Add an edge to the snapshot
    fn add_edge(&mut self, edge: EdgeKey) {
        self.by_source
            .entry(edge.source.clone())
            .or_default()
            .push(edge.clone());
        self.by_target
            .entry(edge.target.clone())
            .or_default()
            .push(edge.clone());
        self.edges.insert(edge);
    }

    /// Get number of edges
    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    /// Check if snapshot contains an edge
    pub fn contains(&self, edge: &EdgeKey) -> bool {
        self.edges.contains(edge)
    }

    /// Get all edges from a source node
    pub fn edges_from(&self, source: &str) -> &[EdgeKey] {
        self.by_source.get(source).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Get all edges to a target node
    pub fn edges_to(&self, target: &str) -> &[EdgeKey] {
        self.by_target.get(target).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Get all edges
    pub fn all_edges(&self) -> impl Iterator<Item = &EdgeKey> {
        self.edges.iter()
    }
}

/// Difference between two edge snapshots
#[derive(Debug, Default)]
pub struct EdgeDiff {
    /// Edges that were added
    pub added: Vec<EdgeKey>,
    /// Edges that were removed
    pub removed: Vec<EdgeKey>,
    /// All nodes affected by edge changes (need re-embedding)
    pub affected_nodes: HashSet<String>,
}

impl EdgeDiff {
    /// Compute the difference between two snapshots
    pub fn compute(before: &EdgeSnapshot, after: &EdgeSnapshot) -> Self {
        let mut diff = Self::default();

        // Find added edges (in after but not in before)
        for edge in after.all_edges() {
            if !before.contains(edge) {
                diff.added.push(edge.clone());
                diff.affected_nodes.insert(edge.source.clone());
                diff.affected_nodes.insert(edge.target.clone());
            }
        }

        // Find removed edges (in before but not in after)
        for edge in before.all_edges() {
            if !after.contains(edge) {
                diff.removed.push(edge.clone());
                diff.affected_nodes.insert(edge.source.clone());
                diff.affected_nodes.insert(edge.target.clone());
            }
        }

        diff
    }

    /// Check if there are any changes
    pub fn has_changes(&self) -> bool {
        !self.added.is_empty() || !self.removed.is_empty()
    }

    /// Get count of added edges
    pub fn added_count(&self) -> usize {
        self.added.len()
    }

    /// Get count of removed edges
    pub fn removed_count(&self) -> usize {
        self.removed.len()
    }

    /// Get count of affected nodes
    pub fn affected_node_count(&self) -> usize {
        self.affected_nodes.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codegraph_db::{DatabaseConnection, run_migrations};

    fn setup_test_db() -> DatabaseConnection {
        let db = DatabaseConnection::open_in_memory().unwrap();
        run_migrations(db.conn()).unwrap();
        db
    }

    fn add_test_node(db: &DatabaseConnection, id: &str, file_path: &str) {
        db.conn()
            .execute(
                "INSERT INTO nodes (id, kind, name, qualified_name, file_path, language,
                 start_line, end_line, start_column, end_column, updated_at)
                 VALUES (?1, 'function', ?1, ?1, ?2, 'typescript', 1, 1, 0, 0, 0)",
                rusqlite::params![id, file_path],
            )
            .unwrap();
    }

    fn add_edge(db: &DatabaseConnection, source: &str, target: &str, kind: &str) {
        db.conn()
            .execute(
                "INSERT INTO edges (source, target, kind) VALUES (?1, ?2, ?3)",
                rusqlite::params![source, target, kind],
            )
            .unwrap();
    }

    fn remove_edge(db: &DatabaseConnection, source: &str, target: &str) {
        db.conn()
            .execute(
                "DELETE FROM edges WHERE source = ?1 AND target = ?2",
                rusqlite::params![source, target],
            )
            .unwrap();
    }

    #[test]
    fn test_edge_snapshot_empty() {
        let db = setup_test_db();
        let snapshot = EdgeSnapshot::capture(db.conn());
        assert_eq!(snapshot.edge_count(), 0);
    }

    #[test]
    fn test_edge_snapshot_capture() {
        let db = setup_test_db();

        add_test_node(&db, "node_a", "a.ts");
        add_test_node(&db, "node_b", "a.ts");
        add_edge(&db, "node_a", "node_b", "calls");

        let snapshot = EdgeSnapshot::capture(db.conn());
        assert_eq!(snapshot.edge_count(), 1);

        let key = EdgeKey::new("node_a", "node_b", "calls");
        assert!(snapshot.contains(&key));
    }

    #[test]
    fn test_edge_snapshot_and_diff() {
        let db = setup_test_db();

        add_test_node(&db, "node_a", "a.ts");
        add_test_node(&db, "node_b", "a.ts");

        // Take snapshot before
        let before = EdgeSnapshot::capture(db.conn());

        // Add edge
        add_edge(&db, "node_a", "node_b", "calls");

        // Take snapshot after
        let after = EdgeSnapshot::capture(db.conn());

        // Compute diff
        let diff = EdgeDiff::compute(&before, &after);

        assert!(diff.has_changes());
        assert_eq!(diff.added_count(), 1);
        assert_eq!(diff.removed_count(), 0);
        assert!(diff.affected_nodes.contains("node_a"));
        assert!(diff.affected_nodes.contains("node_b"));
    }

    #[test]
    fn test_edge_removal_detected() {
        let db = setup_test_db();

        add_test_node(&db, "node_a", "a.ts");
        add_test_node(&db, "node_b", "a.ts");
        add_edge(&db, "node_a", "node_b", "calls");

        let before = EdgeSnapshot::capture(db.conn());

        remove_edge(&db, "node_a", "node_b");

        let after = EdgeSnapshot::capture(db.conn());
        let diff = EdgeDiff::compute(&before, &after);

        assert!(diff.has_changes());
        assert_eq!(diff.added_count(), 0);
        assert_eq!(diff.removed_count(), 1);
        assert!(diff.affected_nodes.contains("node_a"));
        assert!(diff.affected_nodes.contains("node_b"));
    }

    #[test]
    fn test_edge_diff_no_changes() {
        let db = setup_test_db();

        add_test_node(&db, "node_a", "a.ts");
        add_test_node(&db, "node_b", "a.ts");
        add_edge(&db, "node_a", "node_b", "calls");

        let before = EdgeSnapshot::capture(db.conn());
        let after = EdgeSnapshot::capture(db.conn());

        let diff = EdgeDiff::compute(&before, &after);

        assert!(!diff.has_changes());
        assert_eq!(diff.affected_node_count(), 0);
    }

    #[test]
    fn test_edge_diff_multiple_changes() {
        let db = setup_test_db();

        add_test_node(&db, "node_a", "a.ts");
        add_test_node(&db, "node_b", "a.ts");
        add_test_node(&db, "node_c", "a.ts");
        add_edge(&db, "node_a", "node_b", "calls");

        let before = EdgeSnapshot::capture(db.conn());

        // Remove one edge, add another
        remove_edge(&db, "node_a", "node_b");
        add_edge(&db, "node_b", "node_c", "imports");

        let after = EdgeSnapshot::capture(db.conn());
        let diff = EdgeDiff::compute(&before, &after);

        assert!(diff.has_changes());
        assert_eq!(diff.added_count(), 1);
        assert_eq!(diff.removed_count(), 1);
        // All three nodes should be affected
        assert!(diff.affected_nodes.contains("node_a"));
        assert!(diff.affected_nodes.contains("node_b"));
        assert!(diff.affected_nodes.contains("node_c"));
    }

    #[test]
    fn test_snapshot_edges_from() {
        let db = setup_test_db();

        add_test_node(&db, "node_a", "a.ts");
        add_test_node(&db, "node_b", "a.ts");
        add_test_node(&db, "node_c", "a.ts");
        add_edge(&db, "node_a", "node_b", "calls");
        add_edge(&db, "node_a", "node_c", "calls");

        let snapshot = EdgeSnapshot::capture(db.conn());

        let edges = snapshot.edges_from("node_a");
        assert_eq!(edges.len(), 2);
    }

    #[test]
    fn test_snapshot_edges_to() {
        let db = setup_test_db();

        add_test_node(&db, "node_a", "a.ts");
        add_test_node(&db, "node_b", "a.ts");
        add_test_node(&db, "node_c", "a.ts");
        add_edge(&db, "node_a", "node_c", "calls");
        add_edge(&db, "node_b", "node_c", "calls");

        let snapshot = EdgeSnapshot::capture(db.conn());

        let edges = snapshot.edges_to("node_c");
        assert_eq!(edges.len(), 2);
    }

    #[test]
    fn test_capture_for_files() {
        let db = setup_test_db();

        add_test_node(&db, "node_a", "a.ts");
        add_test_node(&db, "node_b", "a.ts");
        add_test_node(&db, "node_c", "b.ts");
        add_edge(&db, "node_a", "node_b", "calls"); // Both in a.ts
        add_edge(&db, "node_b", "node_c", "calls"); // Crosses files

        let snapshot = EdgeSnapshot::capture_for_files(db.conn(), &["a.ts"]);

        // Should capture edges involving nodes in a.ts
        assert!(snapshot.edge_count() >= 1);
    }
}
