//! Selective enrichment scope (L4a, I3)
//!
//! Determines which nodes need re-enrichment based on:
//! - Changed files
//! - Nodes that depend on changed files (via enrichment_deps)
//! - New or changed imports
//! - Transitive dependencies (cascade_depth > 1)

use codegraph_db::{get_nodes_depending_on_files, rusqlite::Connection};
use std::collections::HashSet;

/// Selective scope for determining which files/nodes need re-enrichment
#[derive(Debug, Default)]
pub struct SelectiveScope {
    /// Files that should be re-enriched (changed files)
    files_to_enrich: HashSet<String>,
    /// Nodes that should be re-enriched (dependents of changed files)
    nodes_to_enrich: HashSet<String>,
    /// Whether any new imports were detected
    has_new_imports: bool,
}

impl SelectiveScope {
    /// Create a new empty scope
    pub fn new() -> Self {
        Self::default()
    }

    /// Create scope from a list of changed files
    ///
    /// Files that have been modified, added, or deleted should be re-enriched.
    pub fn from_changed_files(changed_files: &[&str]) -> Self {
        let files_to_enrich: HashSet<String> = changed_files
            .iter()
            .map(|s| s.to_string())
            .collect();

        Self {
            files_to_enrich,
            nodes_to_enrich: HashSet::new(),
            has_new_imports: false,
        }
    }

    /// Create scope from changed files, including nodes that depend on them
    ///
    /// This queries the enrichment_deps table to find all nodes that depend
    /// on the changed files and includes them in the scope.
    pub fn from_changed_files_with_deps(conn: &Connection, changed_files: &[&str]) -> Self {
        let files_to_enrich: HashSet<String> = changed_files
            .iter()
            .map(|s| s.to_string())
            .collect();

        // Find all nodes that depend on any of the changed files
        let nodes_to_enrich = match get_nodes_depending_on_files(conn, changed_files) {
            Ok(nodes) => nodes.into_iter().collect(),
            Err(e) => {
                log::warn!("Failed to query enrichment_deps: {}", e);
                HashSet::new()
            }
        };

        Self {
            files_to_enrich,
            nodes_to_enrich,
            has_new_imports: false,
        }
    }

    /// Create scope from import diff between old and new imports
    ///
    /// Used to detect when a file adds new imports that need enrichment.
    pub fn from_import_diff(old_imports: &[&str], new_imports: &[&str]) -> Self {
        let old_set: HashSet<_> = old_imports.iter().cloned().collect();
        let new_set: HashSet<_> = new_imports.iter().cloned().collect();

        // Find imports that are in new but not in old
        let new_import_count = new_set.difference(&old_set).count();
        let has_new_imports = new_import_count > 0;

        Self {
            files_to_enrich: HashSet::new(),
            nodes_to_enrich: HashSet::new(),
            has_new_imports,
        }
    }

    /// Add a file to the scope
    pub fn add_file(&mut self, file_path: &str) {
        self.files_to_enrich.insert(file_path.to_string());
    }

    /// Add a node to the scope
    pub fn add_node(&mut self, node_id: &str) {
        self.nodes_to_enrich.insert(node_id.to_string());
    }

    /// Add dependent nodes from database
    pub fn add_dependents_from_db(&mut self, conn: &Connection, changed_files: &[&str]) {
        if let Ok(nodes) = get_nodes_depending_on_files(conn, changed_files) {
            for node in nodes {
                self.nodes_to_enrich.insert(node);
            }
        }
    }

    /// Check if a file should be re-enriched
    pub fn should_enrich(&self, file_path: &str) -> bool {
        self.files_to_enrich.contains(file_path)
    }

    /// Check if a node should be re-enriched
    pub fn should_enrich_node(&self, node_id: &str) -> bool {
        self.nodes_to_enrich.contains(node_id)
    }

    /// Check if a file should be re-enriched
    ///
    /// Note: To check if specific nodes need re-enrichment, use `should_enrich_node`.
    /// This method only checks file-level changes.
    pub fn should_enrich_file_or_nodes(&self, file_path: &str) -> bool {
        // Check if the file itself changed
        if self.files_to_enrich.contains(file_path) {
            return true;
        }

        // If there are no dependent nodes, nothing more to check
        // The actual node-level check requires the caller to check each node
        // via should_enrich_node() since we don't track node->file mapping here
        false
    }

    /// Check if any nodes are pending re-enrichment
    pub fn has_nodes_to_enrich(&self) -> bool {
        !self.nodes_to_enrich.is_empty()
    }

    /// Check if there are new imports that need enrichment
    pub fn has_new_imports(&self) -> bool {
        self.has_new_imports
    }

    /// Set the new imports flag
    pub fn set_has_new_imports(&mut self, value: bool) {
        self.has_new_imports = value;
    }

    /// Get the number of files in scope
    pub fn file_count(&self) -> usize {
        self.files_to_enrich.len()
    }

    /// Get the number of nodes in scope
    pub fn node_count(&self) -> usize {
        self.nodes_to_enrich.len()
    }

    /// Check if scope is empty (no files or nodes to enrich)
    pub fn is_empty(&self) -> bool {
        self.files_to_enrich.is_empty() && self.nodes_to_enrich.is_empty() && !self.has_new_imports
    }

    /// Get all files in scope
    pub fn files(&self) -> impl Iterator<Item = &str> {
        self.files_to_enrich.iter().map(|s| s.as_str())
    }

    /// Get all nodes in scope
    pub fn nodes(&self) -> impl Iterator<Item = &str> {
        self.nodes_to_enrich.iter().map(|s| s.as_str())
    }

    /// Merge another scope into this one
    pub fn merge(&mut self, other: SelectiveScope) {
        self.files_to_enrich.extend(other.files_to_enrich);
        self.nodes_to_enrich.extend(other.nodes_to_enrich);
        self.has_new_imports = self.has_new_imports || other.has_new_imports;
    }

    /// Create scope from changed files with cascade depth (I3)
    ///
    /// cascade_depth controls how many levels of transitive dependencies to include:
    /// - 1: Only direct dependents of changed files
    /// - 2: Dependents + dependents of those dependents' files
    /// - N: N levels deep
    pub fn from_changed_files_with_cascade(
        conn: &Connection,
        changed_files: &[&str],
        cascade_depth: u32,
    ) -> Self {
        let files_to_enrich: HashSet<String> = changed_files
            .iter()
            .map(|s| s.to_string())
            .collect();

        // Find affected nodes with cascade
        let nodes_to_enrich = find_affected_nodes_with_cascade(conn, changed_files, cascade_depth);

        Self {
            files_to_enrich,
            nodes_to_enrich,
            has_new_imports: false,
        }
    }
}

/// Find all nodes affected by changed files, respecting cascade_depth (I3)
///
/// Returns a set of node IDs that need re-enrichment.
pub fn find_affected_nodes_with_cascade(
    conn: &Connection,
    changed_files: &[&str],
    cascade_depth: u32,
) -> HashSet<String> {
    if cascade_depth == 0 {
        return HashSet::new();
    }

    let mut all_affected_nodes: HashSet<String> = HashSet::new();
    let mut current_files: HashSet<String> = changed_files.iter().map(|s| s.to_string()).collect();
    let mut visited_files: HashSet<String> = current_files.clone();

    for _depth in 0..cascade_depth {
        // Find nodes depending on current files
        let file_refs: Vec<&str> = current_files.iter().map(|s| s.as_str()).collect();
        let nodes = match get_nodes_depending_on_files(conn, &file_refs) {
            Ok(nodes) => nodes,
            Err(e) => {
                log::warn!("Failed to query enrichment_deps at depth: {}", e);
                break;
            }
        };

        if nodes.is_empty() {
            break;
        }

        // Add nodes to affected set
        all_affected_nodes.extend(nodes.iter().cloned());

        // Get the files containing these nodes for next iteration
        let next_files = match get_files_for_nodes(conn, &nodes) {
            Ok(files) => files,
            Err(e) => {
                log::warn!("Failed to get files for nodes: {}", e);
                break;
            }
        };

        // Filter to only new files (avoid cycles)
        current_files = next_files
            .into_iter()
            .filter(|f| !visited_files.contains(f))
            .collect();

        if current_files.is_empty() {
            break;
        }

        visited_files.extend(current_files.iter().cloned());
    }

    all_affected_nodes
}

/// Get the file paths for a set of nodes
fn get_files_for_nodes(conn: &Connection, node_ids: &[String]) -> Result<HashSet<String>, rusqlite::Error> {
    if node_ids.is_empty() {
        return Ok(HashSet::new());
    }

    let mut files = HashSet::new();

    // Query in batches to avoid SQL parameter limits
    const BATCH_SIZE: usize = 500;
    for chunk in node_ids.chunks(BATCH_SIZE) {
        let placeholders: String = (1..=chunk.len())
            .map(|i| format!("?{}", i))
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT DISTINCT file_path FROM nodes WHERE id IN ({})",
            placeholders
        );

        let mut stmt = conn.prepare(&sql)?;
        let params: Vec<&dyn rusqlite::ToSql> = chunk
            .iter()
            .map(|s| s as &dyn rusqlite::ToSql)
            .collect();

        let rows = stmt.query_map(params.as_slice(), |row| row.get::<_, String>(0))?;
        for file in rows.flatten() {
            files.insert(file);
        }
    }

    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_selective_scope_includes_changed_files() {
        let changed_files = vec!["src/api.ts", "src/utils.ts"];
        let scope = SelectiveScope::from_changed_files(&changed_files);

        assert!(scope.should_enrich("src/api.ts"));
        assert!(scope.should_enrich("src/utils.ts"));
        assert!(!scope.should_enrich("src/other.ts"));
    }

    #[test]
    fn test_selective_scope_empty() {
        let scope = SelectiveScope::new();

        assert!(scope.is_empty());
        assert_eq!(scope.file_count(), 0);
        assert_eq!(scope.node_count(), 0);
    }

    #[test]
    fn test_selective_scope_add_file() {
        let mut scope = SelectiveScope::new();
        scope.add_file("src/new.ts");

        assert!(scope.should_enrich("src/new.ts"));
        assert_eq!(scope.file_count(), 1);
    }

    #[test]
    fn test_selective_scope_add_node() {
        let mut scope = SelectiveScope::new();
        scope.add_node("node_123");

        assert!(scope.should_enrich_node("node_123"));
        assert_eq!(scope.node_count(), 1);
    }

    #[test]
    fn test_selective_scope_includes_new_imports() {
        // L4a: selective scope includes new/changed imports
        let old_imports = vec!["./utils"];
        let new_imports = vec!["./utils", "./newModule"];

        let scope = SelectiveScope::from_import_diff(&old_imports, &new_imports);

        // Nodes with new imports should be flagged
        assert!(scope.has_new_imports());
    }

    #[test]
    fn test_selective_scope_no_new_imports() {
        let old_imports = vec!["./utils", "./types"];
        let new_imports = vec!["./utils"]; // Removed one, no new ones

        let scope = SelectiveScope::from_import_diff(&old_imports, &new_imports);

        assert!(!scope.has_new_imports());
    }

    #[test]
    fn test_selective_scope_merge() {
        let mut scope1 = SelectiveScope::from_changed_files(&["a.ts"]);
        let mut scope2 = SelectiveScope::from_changed_files(&["b.ts"]);
        scope2.add_node("node_1");
        scope2.set_has_new_imports(true);

        scope1.merge(scope2);

        assert!(scope1.should_enrich("a.ts"));
        assert!(scope1.should_enrich("b.ts"));
        assert!(scope1.should_enrich_node("node_1"));
        assert!(scope1.has_new_imports());
    }

    #[test]
    fn test_selective_scope_files_iterator() {
        let changed_files = vec!["a.ts", "b.ts", "c.ts"];
        let scope = SelectiveScope::from_changed_files(&changed_files);

        let files: Vec<_> = scope.files().collect();
        assert_eq!(files.len(), 3);
    }

    #[test]
    fn test_selective_scope_nodes_iterator() {
        let mut scope = SelectiveScope::new();
        scope.add_node("node_1");
        scope.add_node("node_2");

        let nodes: Vec<_> = scope.nodes().collect();
        assert_eq!(nodes.len(), 2);
    }

    // Tests for cascade depth functionality require database setup
    mod cascade_tests {
        use super::*;
        use codegraph_db::{insert_enrichment_dep, run_migrations, DatabaseConnection};

        fn setup_test_db_with_deps() -> DatabaseConnection {
            let db = DatabaseConnection::open_in_memory().unwrap();
            run_migrations(db.conn()).unwrap();

            // Create nodes: a.ts -> b.ts -> c.ts chain
            // node_a depends on b.ts
            // node_b depends on c.ts
            db.conn()
                .execute(
                    "INSERT INTO nodes (id, kind, name, qualified_name, file_path, language,
                     start_line, end_line, start_column, end_column, updated_at)
                     VALUES ('node_a', 'function', 'funcA', 'funcA', 'a.ts', 'typescript', 1, 1, 0, 0, 0)",
                    [],
                )
                .unwrap();
            db.conn()
                .execute(
                    "INSERT INTO nodes (id, kind, name, qualified_name, file_path, language,
                     start_line, end_line, start_column, end_column, updated_at)
                     VALUES ('node_b', 'function', 'funcB', 'funcB', 'b.ts', 'typescript', 1, 1, 0, 0, 0)",
                    [],
                )
                .unwrap();
            db.conn()
                .execute(
                    "INSERT INTO nodes (id, kind, name, qualified_name, file_path, language,
                     start_line, end_line, start_column, end_column, updated_at)
                     VALUES ('node_c', 'function', 'funcC', 'funcC', 'c.ts', 'typescript', 1, 1, 0, 0, 0)",
                    [],
                )
                .unwrap();

            // Set up dependencies: node_a depends on b.ts, node_b depends on c.ts
            insert_enrichment_dep(db.conn(), "node_a", "b.ts").unwrap();
            insert_enrichment_dep(db.conn(), "node_b", "c.ts").unwrap();

            db
        }

        #[test]
        fn test_cascade_depth_zero_returns_empty() {
            let db = setup_test_db_with_deps();

            let affected = find_affected_nodes_with_cascade(db.conn(), &["c.ts"], 0);
            assert!(affected.is_empty());
        }

        #[test]
        fn test_cascade_finds_direct_dependents_only() {
            let db = setup_test_db_with_deps();
            // c.ts changed, node_b depends on c.ts

            let affected = find_affected_nodes_with_cascade(db.conn(), &["c.ts"], 1);

            // Depth 1: Should only find node_b (direct dependent of c.ts)
            assert!(affected.contains("node_b"), "Should find node_b");
            assert!(!affected.contains("node_a"), "Should NOT find node_a at depth 1");
        }

        #[test]
        fn test_cascade_depth_two_finds_transitive() {
            let db = setup_test_db_with_deps();
            // c.ts changed
            // Depth 1: node_b depends on c.ts
            // Depth 2: node_a depends on b.ts (where node_b lives)

            let affected = find_affected_nodes_with_cascade(db.conn(), &["c.ts"], 2);

            // Depth 2 should find both node_b and node_a
            assert!(affected.contains("node_b"), "Should find node_b");
            assert!(affected.contains("node_a"), "Should find node_a at depth 2");
        }

        #[test]
        fn test_cascade_with_no_dependents() {
            let db = setup_test_db_with_deps();

            // a.ts has no dependents
            let affected = find_affected_nodes_with_cascade(db.conn(), &["a.ts"], 3);

            assert!(affected.is_empty(), "a.ts has no dependents");
        }

        #[test]
        fn test_cascade_handles_cycles() {
            let db = DatabaseConnection::open_in_memory().unwrap();
            run_migrations(db.conn()).unwrap();

            // Create circular dependency: a -> b -> a
            db.conn()
                .execute(
                    "INSERT INTO nodes (id, kind, name, qualified_name, file_path, language,
                     start_line, end_line, start_column, end_column, updated_at)
                     VALUES ('node_a', 'function', 'funcA', 'funcA', 'a.ts', 'typescript', 1, 1, 0, 0, 0)",
                    [],
                )
                .unwrap();
            db.conn()
                .execute(
                    "INSERT INTO nodes (id, kind, name, qualified_name, file_path, language,
                     start_line, end_line, start_column, end_column, updated_at)
                     VALUES ('node_b', 'function', 'funcB', 'funcB', 'b.ts', 'typescript', 1, 1, 0, 0, 0)",
                    [],
                )
                .unwrap();

            insert_enrichment_dep(db.conn(), "node_a", "b.ts").unwrap();
            insert_enrichment_dep(db.conn(), "node_b", "a.ts").unwrap();

            // Should not infinite loop even with high depth
            let affected = find_affected_nodes_with_cascade(db.conn(), &["a.ts"], 10);

            // Should find both nodes without hanging
            assert!(affected.contains("node_b"));
            // node_a would be found in depth 2 when we look at dependents of b.ts
        }

        #[test]
        fn test_selective_scope_with_cascade() {
            let db = setup_test_db_with_deps();

            let scope = SelectiveScope::from_changed_files_with_cascade(db.conn(), &["c.ts"], 2);

            // Should include the changed file
            assert!(scope.should_enrich("c.ts"));
            // Should include cascade-affected nodes
            assert!(scope.should_enrich_node("node_b"));
            assert!(scope.should_enrich_node("node_a"));
        }
    }
}
