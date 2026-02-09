//! Reference resolver for CodeGraph
//!
//! Resolves unresolved references after extraction by:
//! 1. Filtering out built-in symbols
//! 2. Import resolution (following import chains)
//! 3. Name matching (exact, qualified, fuzzy)
//! 4. Framework-specific patterns

use crate::error::ResolutionError;
use crate::matcher::NameMatcher;
use codegraph_db::QueryBuilder;
use codegraph_types::{Edge, EdgeKind, Node, NodeId, NodeKind};
use rusqlite::Connection;

/// Statistics from resolution process
#[derive(Debug, Default, Clone)]
pub struct ResolutionStats {
    /// Total unresolved references processed
    pub total_processed: usize,
    /// Successfully resolved
    pub resolved: usize,
    /// Skipped (built-in symbols)
    pub skipped_builtins: usize,
    /// Ambiguous (multiple candidates)
    pub ambiguous: usize,
    /// Unresolved (no candidates)
    pub unresolved: usize,
}

impl ResolutionStats {
    /// Get resolution success rate
    pub fn success_rate(&self) -> f64 {
        if self.total_processed == 0 {
            return 0.0;
        }
        self.resolved as f64 / self.total_processed as f64
    }
}

/// Result of resolving a single reference
#[derive(Debug)]
pub struct ResolutionResult {
    /// The original unresolved reference name
    pub reference_name: String,
    /// The source node that made the reference
    pub source_id: NodeId,
    /// The resolved target node, if found
    pub target: Option<ResolvedTarget>,
    /// Why it wasn't resolved (if applicable)
    pub reason: Option<String>,
}

/// A resolved target
#[derive(Debug, Clone)]
pub struct ResolvedTarget {
    /// The target node ID
    pub node_id: NodeId,
    /// Confidence score (0.0 to 1.0)
    pub confidence: f64,
    /// How it was resolved
    pub method: ResolutionMethod,
}

/// How a reference was resolved
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolutionMethod {
    /// Direct import chain
    Import,
    /// Exact name match
    ExactMatch,
    /// Qualified name suffix match
    QualifiedMatch,
    /// Fuzzy name match
    FuzzyMatch,
    /// Framework-specific pattern
    Framework,
}

/// Configuration for the resolver
#[derive(Debug, Clone)]
pub struct ResolverConfig {
    /// Enable fuzzy matching
    pub fuzzy_matching: bool,
    /// Minimum confidence for fuzzy matches
    pub fuzzy_threshold: f64,
    /// Maximum candidates to consider
    pub max_candidates: usize,
    /// Enable framework-specific resolution
    pub framework_resolution: bool,
}

impl Default for ResolverConfig {
    fn default() -> Self {
        Self {
            fuzzy_matching: true,
            fuzzy_threshold: 0.8,
            max_candidates: 10,
            framework_resolution: true,
        }
    }
}

/// Reference resolver
pub struct ReferenceResolver<'a> {
    conn: &'a Connection,
    queries: &'a mut QueryBuilder,
    matcher: NameMatcher,
    config: ResolverConfig,
    stats: ResolutionStats,
}

impl<'a> ReferenceResolver<'a> {
    /// Create a new resolver
    pub fn new(conn: &'a Connection, queries: &'a mut QueryBuilder) -> Self {
        Self {
            conn,
            queries,
            matcher: NameMatcher::new(),
            config: ResolverConfig::default(),
            stats: ResolutionStats::default(),
        }
    }

    /// Create with custom configuration
    pub fn with_config(
        conn: &'a Connection,
        queries: &'a mut QueryBuilder,
        config: ResolverConfig,
    ) -> Self {
        Self {
            conn,
            queries,
            matcher: NameMatcher::new(),
            config,
            stats: ResolutionStats::default(),
        }
    }

    /// Get current stats
    pub fn stats(&self) -> &ResolutionStats {
        &self.stats
    }

    /// Resolve all unresolved references in the database
    pub fn resolve_all(&mut self) -> Result<ResolutionStats, ResolutionError> {
        let unresolved = self.queries.get_all_unresolved_refs(self.conn)?;

        log::info!(
            "Starting resolution of {} unresolved references",
            unresolved.len()
        );

        for unresolved_ref in unresolved {
            let result = self.resolve_reference_with_kind(
                unresolved_ref.from_node_id.as_str(),
                &unresolved_ref.reference_name,
                unresolved_ref.reference_kind,
            )?;
            self.stats.total_processed += 1;

            if result.target.is_some() {
                self.stats.resolved += 1;
            } else if result
                .reason
                .as_ref()
                .map(|r| r.contains("builtin"))
                .unwrap_or(false)
            {
                self.stats.skipped_builtins += 1;
            } else if result
                .reason
                .as_ref()
                .map(|r| r.contains("ambiguous"))
                .unwrap_or(false)
            {
                self.stats.ambiguous += 1;
            } else {
                self.stats.unresolved += 1;
            }
        }

        log::info!(
            "Resolution complete: {} resolved, {} skipped, {} ambiguous, {} unresolved",
            self.stats.resolved,
            self.stats.skipped_builtins,
            self.stats.ambiguous,
            self.stats.unresolved
        );

        Ok(self.stats.clone())
    }

    /// Resolve a single reference (defaults to EdgeKind::References for backward compat)
    pub fn resolve_reference(
        &mut self,
        source_id: &str,
        ref_name: &str,
    ) -> Result<ResolutionResult, ResolutionError> {
        self.resolve_reference_with_kind(source_id, ref_name, EdgeKind::References)
    }

    /// Resolve a single reference with a specific edge kind
    pub fn resolve_reference_with_kind(
        &mut self,
        source_id: &str,
        ref_name: &str,
        edge_kind: EdgeKind,
    ) -> Result<ResolutionResult, ResolutionError> {
        let source_node_id = NodeId::new(source_id);

        // Skip built-in symbols
        if self.matcher.is_builtin(ref_name) {
            return Ok(ResolutionResult {
                reference_name: ref_name.to_string(),
                source_id: source_node_id,
                target: None,
                reason: Some("Skipped builtin symbol".to_string()),
            });
        }

        // Get the source node to understand context
        let source_node = self.queries.get_node_by_id(self.conn, source_id)?;

        // Get source file path for scope-aware resolution
        let source_file = source_node
            .as_ref()
            .map(|n| n.file_path.as_str())
            .unwrap_or("");

        // Try resolution strategies in order
        let target = self
            .try_import_resolution(source_id, ref_name)?
            .or_else(|| self.try_exact_match(ref_name, source_file).ok().flatten())
            .or_else(|| self.try_qualified_match(ref_name, source_file).ok().flatten())
            .or_else(|| {
                if self.config.fuzzy_matching {
                    self.try_fuzzy_match(ref_name).ok().flatten()
                } else {
                    None
                }
            })
            .or_else(|| {
                if self.config.framework_resolution {
                    self.try_framework_resolution(source_node.as_ref(), ref_name)
                        .ok()
                        .flatten()
                } else {
                    None
                }
            });

        // If resolved, create the edge with the actual reference kind
        if let Some(ref resolved) = target {
            let edge = Edge::new(source_id, resolved.node_id.as_str(), edge_kind);
            // Ignore duplicate edge errors
            let _ = self.queries.insert_edge(self.conn, &edge);

            // Mark as resolved in unresolved_refs table
            self.queries
                .delete_unresolved_reference(self.conn, source_id, ref_name)?;
        }

        Ok(ResolutionResult {
            reference_name: ref_name.to_string(),
            source_id: source_node_id,
            target,
            reason: None,
        })
    }

    /// Try to resolve via import chain
    fn try_import_resolution(
        &mut self,
        source_id: &str,
        ref_name: &str,
    ) -> Result<Option<ResolvedTarget>, ResolutionError> {
        // Get the file containing the source node
        let source_node = match self.queries.get_node_by_id(self.conn, source_id)? {
            Some(n) => n,
            None => return Ok(None),
        };

        // Find imports in the same file
        let file_nodes = self
            .queries
            .get_nodes_in_file(self.conn, &source_node.file_path)?;
        let import_nodes: Vec<_> = file_nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Import)
            .collect();

        for import in import_nodes {
            // Check if the import name matches what we're looking for
            if import.name == ref_name || import.name.ends_with(&format!("::{}", ref_name)) {
                // Follow the import to find the actual target
                let imported_edges =
                    self.queries
                        .get_outgoing_edges(self.conn, &import.id.0, None)?;

                for edge in imported_edges {
                    if edge.kind == EdgeKind::Imports {
                        return Ok(Some(ResolvedTarget {
                            node_id: edge.target.clone(),
                            confidence: 1.0,
                            method: ResolutionMethod::Import,
                        }));
                    }
                }
            }
        }

        Ok(None)
    }

    /// Try exact name match with scope-aware resolution.
    /// Prefers same-file matches; skips ambiguous cross-file matches for common names.
    fn try_exact_match(
        &self,
        ref_name: &str,
        source_file: &str,
    ) -> Result<Option<ResolvedTarget>, ResolutionError> {
        let candidates = self.queries.search_symbols(self.conn, ref_name)?;

        // Collect all exact matches, partitioned by file proximity
        let mut same_file = Vec::new();
        let mut other_file = Vec::new();
        for node in &candidates {
            if node.name == ref_name {
                if node.file_path == source_file {
                    same_file.push(node);
                } else {
                    other_file.push(node);
                }
            }
        }

        // Prefer same-file match (highest confidence)
        if same_file.len() == 1 {
            return Ok(Some(ResolvedTarget {
                node_id: same_file[0].id.clone(),
                confidence: 1.0,
                method: ResolutionMethod::ExactMatch,
            }));
        }

        // If exactly one cross-file match, return it
        if same_file.is_empty() && other_file.len() == 1 {
            return Ok(Some(ResolvedTarget {
                node_id: other_file[0].id.clone(),
                confidence: 0.9,
                method: ResolutionMethod::ExactMatch,
            }));
        }

        // Multiple matches = ambiguous, skip to avoid false positives
        Ok(None)
    }

    /// Try qualified name suffix match with scope-aware resolution.
    fn try_qualified_match(
        &self,
        ref_name: &str,
        source_file: &str,
    ) -> Result<Option<ResolvedTarget>, ResolutionError> {
        let candidates = self.queries.search_symbols(self.conn, ref_name)?;

        let mut same_file = Vec::new();
        let mut other_file = Vec::new();
        for node in &candidates {
            if self.matcher.qualified_suffix_match(ref_name, &node.qualified_name) {
                if node.file_path == source_file {
                    same_file.push(node);
                } else {
                    other_file.push(node);
                }
            }
        }

        // Prefer same-file match
        if same_file.len() == 1 {
            return Ok(Some(ResolvedTarget {
                node_id: same_file[0].id.clone(),
                confidence: 0.95,
                method: ResolutionMethod::QualifiedMatch,
            }));
        }

        // Single cross-file match
        if same_file.is_empty() && other_file.len() == 1 {
            return Ok(Some(ResolvedTarget {
                node_id: other_file[0].id.clone(),
                confidence: 0.85,
                method: ResolutionMethod::QualifiedMatch,
            }));
        }

        Ok(None)
    }

    /// Try fuzzy name match
    fn try_fuzzy_match(&self, ref_name: &str) -> Result<Option<ResolvedTarget>, ResolutionError> {
        let candidates = self.queries.search_symbols(self.conn, ref_name)?;

        if candidates.is_empty() {
            return Ok(None);
        }

        let names: Vec<&str> = candidates.iter().map(|n| n.name.as_str()).collect();
        let matches = self
            .matcher
            .find_all_matches(ref_name, &names, self.config.fuzzy_threshold);

        if let Some((matched_name, score)) = matches.first() {
            // Find the node with this name
            for node in &candidates {
                if node.name == *matched_name {
                    return Ok(Some(ResolvedTarget {
                        node_id: node.id.clone(),
                        confidence: *score,
                        method: ResolutionMethod::FuzzyMatch,
                    }));
                }
            }
        }

        Ok(None)
    }

    /// Try framework-specific resolution patterns
    fn try_framework_resolution(
        &self,
        source_node: Option<&Node>,
        ref_name: &str,
    ) -> Result<Option<ResolvedTarget>, ResolutionError> {
        let source = match source_node {
            Some(n) => n,
            None => return Ok(None),
        };

        // React: Component references
        if ref_name.chars().next().map(|c| c.is_uppercase()) == Some(true) {
            // Could be a React component - look for components
            let candidates = self.queries.search_symbols(self.conn, ref_name)?;
            for node in candidates {
                if node.kind == NodeKind::Component && node.name == ref_name {
                    return Ok(Some(ResolvedTarget {
                        node_id: node.id,
                        confidence: 0.9,
                        method: ResolutionMethod::Framework,
                    }));
                }
            }
        }

        // Express/Flask/etc: Route handlers
        if source.kind == NodeKind::Route {
            // Look for handler functions
            let candidates = self.queries.search_symbols(self.conn, ref_name)?;
            for node in candidates {
                if node.kind == NodeKind::Function && node.name == ref_name {
                    return Ok(Some(ResolvedTarget {
                        node_id: node.id,
                        confidence: 0.85,
                        method: ResolutionMethod::Framework,
                    }));
                }
            }
        }

        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codegraph_db::DatabaseConnection;
    use codegraph_types::Language;

    fn create_test_node(id: &str, name: &str, kind: NodeKind) -> Node {
        Node::new(
            id,
            kind,
            name,
            format!("test.rs::{name}"),
            "test.rs",
            Language::Rust,
            1,
            10,
        )
    }

    fn setup_test_db() -> (DatabaseConnection, QueryBuilder) {
        let db = DatabaseConnection::open_in_memory().unwrap();
        let queries = QueryBuilder::new(db.conn()).unwrap();
        (db, queries)
    }

    #[test]
    fn test_resolve_skips_builtins() {
        let (db, mut queries) = setup_test_db();

        // Create a source node
        let source = create_test_node("src1", "main", NodeKind::Function);
        queries.insert_node(db.conn(), &source).unwrap();

        // Add unresolved reference to "console"
        queries
            .insert_unresolved_reference(db.conn(), "src1", "console", "test.rs", 5)
            .unwrap();

        let mut resolver = ReferenceResolver::new(db.conn(), &mut queries);
        let result = resolver.resolve_reference("src1", "console").unwrap();

        assert!(result.target.is_none());
        assert!(result
            .reason
            .as_ref()
            .map(|r| r.contains("builtin"))
            .unwrap_or(false));
    }

    #[test]
    fn test_resolve_exact_match() {
        let (db, mut queries) = setup_test_db();

        // Create source and target nodes
        let source = create_test_node("src1", "caller", NodeKind::Function);
        let target = create_test_node("tgt1", "targetFunc", NodeKind::Function);

        queries.insert_node(db.conn(), &source).unwrap();
        queries.insert_node(db.conn(), &target).unwrap();

        // Add unresolved reference
        queries
            .insert_unresolved_reference(db.conn(), "src1", "targetFunc", "test.rs", 5)
            .unwrap();

        let mut resolver = ReferenceResolver::new(db.conn(), &mut queries);
        let result = resolver.resolve_reference("src1", "targetFunc").unwrap();

        assert!(result.target.is_some());
        let resolved = result.target.unwrap();
        assert_eq!(resolved.node_id.as_str(), "tgt1");
        assert_eq!(resolved.method, ResolutionMethod::ExactMatch);
        assert!((resolved.confidence - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_resolution_stats() {
        let (db, mut queries) = setup_test_db();

        // Create nodes
        let source = create_test_node("src1", "main", NodeKind::Function);
        let target = create_test_node("tgt1", "helper", NodeKind::Function);

        queries.insert_node(db.conn(), &source).unwrap();
        queries.insert_node(db.conn(), &target).unwrap();

        // Add various unresolved references
        queries
            .insert_unresolved_reference(db.conn(), "src1", "helper", "test.rs", 5)
            .unwrap();
        queries
            .insert_unresolved_reference(db.conn(), "src1", "console", "test.rs", 6)
            .unwrap();
        queries
            .insert_unresolved_reference(db.conn(), "src1", "nonexistent", "test.rs", 7)
            .unwrap();

        let mut resolver = ReferenceResolver::new(db.conn(), &mut queries);
        let stats = resolver.resolve_all().unwrap();

        assert_eq!(stats.total_processed, 3);
        assert_eq!(stats.resolved, 1); // "helper"
        assert_eq!(stats.skipped_builtins, 1); // "console"
        assert_eq!(stats.unresolved, 1); // "nonexistent"
    }

    #[test]
    fn test_resolver_config() {
        let config = ResolverConfig {
            fuzzy_matching: false,
            fuzzy_threshold: 0.9,
            max_candidates: 5,
            framework_resolution: false,
        };

        let (db, mut queries) = setup_test_db();
        let resolver = ReferenceResolver::with_config(db.conn(), &mut queries, config);

        assert!(!resolver.config.fuzzy_matching);
        assert!((resolver.config.fuzzy_threshold - 0.9).abs() < 0.001);
    }
}
