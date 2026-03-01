//! Pre-delete impact capture for embedding candidate computation.
//!
//! Captures IDs of nodes affected by deleting/modifying a set of nodes,
//! BEFORE those nodes are deleted (while edges still exist in the DB).
//! Replaces EdgeSnapshot on the hot path with targeted queries proportional
//! to changed files, not all edges.

use codegraph_db::rusqlite::Connection;
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
    let offset = ids.len();

    let sql = format!(
        r#"SELECT DISTINCT e.source AS affected_id FROM edges e
           WHERE e.target IN ({ph}) AND e.kind IN ('calls','extends','implements','references')
           UNION
           SELECT DISTINCT e.target AS affected_id FROM edges e
           WHERE e.source IN ({second_ph}) AND e.kind IN ('calls','extends','implements','references')"#,
        ph = ph,
        second_ph = (1..=ids.len())
            .map(|i| format!("?{}", i + offset))
            .collect::<Vec<_>>()
            .join(","),
    );

    let mut stmt = conn.prepare(&sql)?;
    let double_params: Vec<Box<dyn rusqlite::ToSql>> = ids
        .iter()
        .chain(ids.iter())
        .map(|id| Box::new(id.to_string()) as Box<dyn rusqlite::ToSql>)
        .collect();
    let param_refs: Vec<&dyn rusqlite::ToSql> = double_params.iter().map(|p| p.as_ref()).collect();

    let results = stmt
        .query_map(param_refs.as_slice(), |row| row.get::<_, String>(0))?
        .collect::<Result<HashSet<String>, _>>()?;

    Ok(results)
}

/// Query nodes that share a Contains parent with the given nodes.
fn query_contains_siblings(conn: &Connection, ids: &[&str]) -> Result<HashSet<String>, DbError> {
    let placeholders: Vec<String> = (1..=ids.len()).map(|i| format!("?{}", i)).collect();
    let ph = placeholders.join(",");
    let offset = ids.len();

    let sql = format!(
        r#"SELECT DISTINCT e2.target AS sibling_id
           FROM edges e1
           JOIN edges e2 ON e1.source = e2.source AND e2.kind = 'contains'
           WHERE e1.target IN ({ph}) AND e1.kind = 'contains'
             AND e2.target NOT IN ({second_ph})"#,
        ph = ph,
        second_ph = (1..=ids.len())
            .map(|i| format!("?{}", i + offset))
            .collect::<Vec<_>>()
            .join(","),
    );

    let mut stmt = conn.prepare(&sql)?;
    let double_params: Vec<Box<dyn rusqlite::ToSql>> = ids
        .iter()
        .chain(ids.iter())
        .map(|id| Box::new(id.to_string()) as Box<dyn rusqlite::ToSql>)
        .collect();
    let param_refs: Vec<&dyn rusqlite::ToSql> = double_params.iter().map(|p| p.as_ref()).collect();

    let results = stmt
        .query_map(param_refs.as_slice(), |row| row.get::<_, String>(0))?
        .collect::<Result<HashSet<String>, _>>()?;

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use codegraph_db::DatabaseConnection;

    fn setup_test_db() -> DatabaseConnection {
        let db = DatabaseConnection::open_in_memory().unwrap();
        for (id, file) in [("a", "a.rs"), ("b", "a.rs"), ("c", "b.rs"), ("parent", "a.rs")] {
            db.conn()
                .execute(
                    "INSERT INTO nodes (id, kind, name, qualified_name, file_path, language,
                     start_line, end_line, start_column, end_column, updated_at)
                     VALUES (?1, 'function', ?1, ?1, ?2, 'rust', 1, 1, 0, 0, 0)",
                    rusqlite::params![id, file],
                )
                .unwrap();
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
        db.conn()
            .execute(
                "INSERT INTO edges (source, target, kind) VALUES ('c', 'a', 'calls')",
                [],
            )
            .unwrap();

        let impact = ImpactCapture::capture(db.conn(), &["a"]).unwrap();
        assert!(impact.affected_ids.contains("c"));
        assert!(!impact.affected_ids.contains("a"));
    }

    #[test]
    fn test_impact_capture_reverse_direction() {
        let db = setup_test_db();
        db.conn()
            .execute(
                "INSERT INTO edges (source, target, kind) VALUES ('a', 'c', 'calls')",
                [],
            )
            .unwrap();

        // When deleting 'a', 'c' should be affected (it was called by 'a')
        let impact = ImpactCapture::capture(db.conn(), &["a"]).unwrap();
        assert!(impact.affected_ids.contains("c"));
        assert!(!impact.affected_ids.contains("a"));
    }

    #[test]
    fn test_impact_capture_siblings() {
        let db = setup_test_db();
        db.conn()
            .execute(
                "INSERT INTO edges (source, target, kind) VALUES ('parent', 'a', 'contains')",
                [],
            )
            .unwrap();
        db.conn()
            .execute(
                "INSERT INTO edges (source, target, kind) VALUES ('parent', 'b', 'contains')",
                [],
            )
            .unwrap();

        let impact = ImpactCapture::capture(db.conn(), &["a"]).unwrap();
        assert!(impact.sibling_ids.contains("b"));
        assert!(!impact.sibling_ids.contains("a"));
    }

    #[test]
    fn test_impact_capture_merge() {
        let mut a = ImpactCapture::new();
        a.affected_ids.insert("x".to_string());
        a.sibling_ids.insert("y".to_string());

        let mut b = ImpactCapture::new();
        b.affected_ids.insert("z".to_string());
        b.sibling_ids.insert("w".to_string());

        a.merge(b);
        assert!(a.affected_ids.contains("x"));
        assert!(a.affected_ids.contains("z"));
        assert!(a.sibling_ids.contains("y"));
        assert!(a.sibling_ids.contains("w"));
    }

    #[test]
    fn test_impact_ignores_contains_edges_in_neighbors() {
        let db = setup_test_db();
        // A 'contains' edge should NOT appear in affected_ids (only in siblings)
        db.conn()
            .execute(
                "INSERT INTO edges (source, target, kind) VALUES ('parent', 'a', 'contains')",
                [],
            )
            .unwrap();

        let impact = ImpactCapture::capture(db.conn(), &["a"]).unwrap();
        // 'parent' should NOT be in affected_ids (contains is excluded from neighbor query)
        assert!(!impact.affected_ids.contains("parent"));
    }

    #[test]
    fn test_impact_capture_multiple_edge_kinds() {
        let db = setup_test_db();
        db.conn()
            .execute(
                "INSERT INTO edges (source, target, kind) VALUES ('b', 'a', 'extends')",
                [],
            )
            .unwrap();
        db.conn()
            .execute(
                "INSERT INTO edges (source, target, kind, line, col) VALUES ('c', 'a', 'references', 5, 0)",
                [],
            )
            .unwrap();

        let impact = ImpactCapture::capture(db.conn(), &["a"]).unwrap();
        assert!(impact.affected_ids.contains("b"));
        assert!(impact.affected_ids.contains("c"));
    }
}
