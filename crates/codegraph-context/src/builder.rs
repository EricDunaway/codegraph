//! Context builder for AI assistants
//!
//! Builds relevant context from the knowledge graph for AI consumption.

use crate::error::ContextError;
use crate::formatter::{format_subgraph, ContextFormat, ContextFormatter, FormattedContext};
use codegraph_db::QueryBuilder;
use codegraph_graph::{GraphQueryManager, GraphTraverser};
use codegraph_types::{EdgeKind, Node, NodeKind, Subgraph, TraversalDirection, TraversalOptions};
use rusqlite::Connection;
use std::collections::HashSet;
use std::fs;
use std::path::Path;

/// Options for context building
#[derive(Debug, Clone)]
pub struct ContextOptions {
    /// Maximum depth to traverse
    pub max_depth: u32,
    /// Maximum number of nodes to include
    pub max_nodes: usize,
    /// Maximum estimated tokens
    pub max_tokens: usize,
    /// Whether to include source code
    pub include_source: bool,
    /// Maximum lines of source per node
    pub max_source_lines: usize,
    /// Output format
    pub format: ContextFormat,
    /// Include callers (upstream)
    pub include_callers: bool,
    /// Include callees (downstream)
    pub include_callees: bool,
    /// Include type hierarchy
    pub include_types: bool,
    /// Include imports/exports
    pub include_imports: bool,
    /// Node kinds to include (None = all)
    pub node_kinds: Option<Vec<NodeKind>>,
    /// Base path for resolving file paths
    pub base_path: Option<String>,
}

impl Default for ContextOptions {
    fn default() -> Self {
        Self {
            max_depth: 3,
            max_nodes: 50,
            max_tokens: 8000,
            include_source: true,
            max_source_lines: 30,
            format: ContextFormat::Markdown,
            include_callers: true,
            include_callees: true,
            include_types: true,
            include_imports: true,
            node_kinds: None,
            base_path: None,
        }
    }
}

/// Result of context building
#[derive(Debug)]
pub struct ContextResult {
    /// Formatted context
    pub context: FormattedContext,
    /// Formatted output string
    pub output: String,
    /// Whether context was truncated
    pub truncated: bool,
    /// Nodes that were excluded due to limits
    pub excluded_count: usize,
}

/// Context builder
pub struct ContextBuilder<'a> {
    conn: &'a Connection,
    queries: &'a mut QueryBuilder,
    options: ContextOptions,
}

impl<'a> ContextBuilder<'a> {
    /// Create a new context builder
    pub fn new(conn: &'a Connection, queries: &'a mut QueryBuilder) -> Self {
        Self {
            conn,
            queries,
            options: ContextOptions::default(),
        }
    }

    /// Create with custom options
    pub fn with_options(
        conn: &'a Connection,
        queries: &'a mut QueryBuilder,
        options: ContextOptions,
    ) -> Self {
        Self {
            conn,
            queries,
            options,
        }
    }

    /// Build context for a specific node
    pub fn build_for_node(&mut self, node_id: &str, query: &str) -> Result<ContextResult, ContextError> {
        let mut subgraph = Subgraph::default();
        let mut visited: HashSet<String> = HashSet::new();

        // Get the root node
        let root_node = self
            .queries
            .get_node_by_id(self.conn, node_id)?
            .ok_or_else(|| ContextError::NodeNotFound(node_id.to_string()))?;

        subgraph.add_node(root_node.clone());
        subgraph.roots.push(root_node.id.clone());
        visited.insert(node_id.to_string());

        // Build traversal options
        let traversal_opts = TraversalOptions {
            direction: TraversalDirection::Outgoing,
            max_depth: Some(self.options.max_depth),
            limit: Some(self.options.max_nodes),
            include_start: false,
            edge_kinds: self.build_edge_kinds(),
            node_kinds: self.options.node_kinds.clone(),
        };

        // Traverse outgoing (callees)
        if self.options.include_callees {
            let mut traverser = GraphTraverser::new(self.conn, self.queries);
            let result = traverser.bfs(node_id, &traversal_opts)?;
            self.merge_subgraph(&mut subgraph, &result.subgraph, &mut visited);
        }

        // Traverse incoming (callers)
        if self.options.include_callers {
            let caller_opts = TraversalOptions {
                direction: TraversalDirection::Incoming,
                ..traversal_opts.clone()
            };
            let mut traverser = GraphTraverser::new(self.conn, self.queries);
            let result = traverser.bfs(node_id, &caller_opts)?;
            self.merge_subgraph(&mut subgraph, &result.subgraph, &mut visited);
        }

        // Get type hierarchy if applicable
        if self.options.include_types {
            if matches!(
                root_node.kind,
                NodeKind::Class | NodeKind::Struct | NodeKind::Interface | NodeKind::Trait
            ) {
                let mut query_manager = GraphQueryManager::new(self.conn, self.queries);
                let hierarchy = query_manager.get_type_hierarchy(node_id)?;
                self.merge_subgraph(&mut subgraph, &hierarchy, &mut visited);
            }
        }

        self.finalize_context(subgraph, query)
    }

    /// Build context for a search query
    pub fn build_for_query(&mut self, search_query: &str) -> Result<ContextResult, ContextError> {
        let mut subgraph = Subgraph::default();
        let mut visited: HashSet<String> = HashSet::new();

        // Search for matching nodes
        let results = self.queries.search_nodes(
            self.conn,
            search_query,
            self.options.node_kinds.as_deref(),
            None,
            self.options.max_nodes,
            0,
        )?;

        for result in results {
            if visited.contains(&result.node.id.0) {
                continue;
            }

            subgraph.add_node(result.node.clone());
            subgraph.roots.push(result.node.id.clone());
            visited.insert(result.node.id.0.clone());

            // Optionally expand each result
            if self.options.include_callees || self.options.include_callers {
                let traversal_opts = TraversalOptions {
                    direction: if self.options.include_callees {
                        TraversalDirection::Both
                    } else {
                        TraversalDirection::Incoming
                    },
                    max_depth: Some(1), // Shallow expansion for search results
                    limit: Some(5),
                    include_start: false,
                    edge_kinds: self.build_edge_kinds(),
                    node_kinds: self.options.node_kinds.clone(),
                };

                let mut traverser = GraphTraverser::new(self.conn, self.queries);
                if let Ok(result) = traverser.bfs(&result.node.id.0, &traversal_opts) {
                    self.merge_subgraph(&mut subgraph, &result.subgraph, &mut visited);
                }
            }
        }

        self.finalize_context(subgraph, search_query)
    }

    /// Build context for a file
    pub fn build_for_file(&mut self, file_path: &str) -> Result<ContextResult, ContextError> {
        let mut subgraph = Subgraph::default();
        let visited: HashSet<String> = HashSet::new();

        // Get all nodes in the file
        let nodes = self.queries.get_nodes_by_file(self.conn, file_path)?;

        for node in nodes {
            subgraph.add_node(node);
        }

        let query = format!("File: {}", file_path);
        self.finalize_context(subgraph, &query)
    }

    /// Build context for impact analysis
    pub fn build_for_impact(
        &mut self,
        node_id: &str,
        max_depth: u32,
    ) -> Result<ContextResult, ContextError> {
        let mut query_manager = GraphQueryManager::new(self.conn, self.queries);
        let impact = query_manager.get_impact_radius(node_id, max_depth)?;

        let mut subgraph = Subgraph::default();

        // Get root node
        if let Some(root) = self.queries.get_node_by_id(self.conn, node_id)? {
            subgraph.add_node(root.clone());
            subgraph.roots.push(root.id);
        }

        // Add direct impacts
        for node in impact.direct {
            subgraph.add_node(node);
        }

        // Add indirect impacts (up to limit)
        for node in impact.indirect.into_iter().take(self.options.max_nodes) {
            subgraph.add_node(node);
        }

        let query = format!("Impact analysis for: {}", node_id);
        self.finalize_context(subgraph, &query)
    }

    /// Merge a subgraph into the main subgraph
    fn merge_subgraph(
        &self,
        main: &mut Subgraph,
        other: &Subgraph,
        visited: &mut HashSet<String>,
    ) {
        for (id, node) in &other.nodes {
            if !visited.contains(&id.0) {
                visited.insert(id.0.clone());
                main.add_node(node.clone());
            }
        }

        for edge in &other.edges {
            main.add_edge(edge.clone());
        }
    }

    /// Build edge kinds to follow based on options
    fn build_edge_kinds(&self) -> Option<Vec<EdgeKind>> {
        let mut kinds = Vec::new();

        if self.options.include_callees || self.options.include_callers {
            kinds.push(EdgeKind::Calls);
            kinds.push(EdgeKind::References);
        }

        if self.options.include_types {
            kinds.push(EdgeKind::Extends);
            kinds.push(EdgeKind::Implements);
            kinds.push(EdgeKind::TypeOf);
        }

        if self.options.include_imports {
            kinds.push(EdgeKind::Imports);
            kinds.push(EdgeKind::Exports);
        }

        if kinds.is_empty() {
            None
        } else {
            Some(kinds)
        }
    }

    /// Finalize context with formatting and limits
    fn finalize_context(
        &self,
        mut subgraph: Subgraph,
        query: &str,
    ) -> Result<ContextResult, ContextError> {
        let total_nodes = subgraph.node_count();
        let mut truncated = false;
        let mut excluded_count = 0;

        // Apply node limit
        if total_nodes > self.options.max_nodes {
            truncated = true;
            excluded_count = total_nodes - self.options.max_nodes;
            // Keep roots and trim others
            let roots: HashSet<_> = subgraph.roots.iter().map(|id| id.0.clone()).collect();
            let mut to_remove: Vec<_> = subgraph
                .nodes
                .keys()
                .filter(|id| !roots.contains(&id.0))
                .skip(self.options.max_nodes.saturating_sub(roots.len()))
                .cloned()
                .collect();
            for id in to_remove.drain(..) {
                subgraph.nodes.remove(&id);
            }
        }

        // Create source loader
        let base_path = self.options.base_path.clone();
        let max_lines = self.options.max_source_lines;
        let include_source = self.options.include_source;

        let source_loader: Option<Box<dyn Fn(&str, u32, u32) -> Option<String>>> =
            if include_source {
                Some(Box::new(move |file_path: &str, start: u32, end: u32| {
                    load_source_lines(file_path, start, end, max_lines, base_path.as_deref())
                }))
            } else {
                None
            };

        // Format subgraph
        let context = format_subgraph(
            &subgraph,
            query,
            source_loader.as_ref().map(|f| f.as_ref()),
        );

        // Check token limit
        if context.estimated_tokens > self.options.max_tokens {
            truncated = true;
        }

        // Format output
        let formatter = ContextFormatter::new(self.options.format)
            .with_source(self.options.include_source)
            .with_max_source_lines(self.options.max_source_lines);

        let output = formatter.format(&context)?;

        Ok(ContextResult {
            context,
            output,
            truncated,
            excluded_count,
        })
    }
}

/// Load source code lines from a file
fn load_source_lines(
    file_path: &str,
    start_line: u32,
    end_line: u32,
    max_lines: usize,
    base_path: Option<&str>,
) -> Option<String> {
    let full_path = if let Some(base) = base_path {
        Path::new(base).join(file_path)
    } else {
        Path::new(file_path).to_path_buf()
    };

    let content = fs::read_to_string(&full_path).ok()?;
    let lines: Vec<&str> = content.lines().collect();

    let start = (start_line as usize).saturating_sub(1);
    let end = (end_line as usize).min(lines.len());
    let range_len = end.saturating_sub(start);

    if range_len == 0 {
        return None;
    }

    let actual_end = start + range_len.min(max_lines);
    let selected: Vec<&str> = lines[start..actual_end].to_vec();

    if selected.is_empty() {
        None
    } else {
        Some(selected.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codegraph_db::DatabaseConnection;
    use codegraph_types::{Edge, Language};

    fn create_test_node(id: &str, name: &str, kind: NodeKind) -> Node {
        Node::new(
            id,
            kind,
            name,
            format!("test.rs::{}", name),
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
    fn test_context_options_default() {
        let opts = ContextOptions::default();
        assert_eq!(opts.max_depth, 3);
        assert_eq!(opts.max_nodes, 50);
        assert!(opts.include_source);
    }

    #[test]
    fn test_build_for_node() {
        let (db, mut queries) = setup_test_db();

        // Create test nodes
        let main_func = create_test_node("main", "main", NodeKind::Function);
        let helper = create_test_node("helper", "helper", NodeKind::Function);

        queries.insert_node(db.conn(), &main_func).unwrap();
        queries.insert_node(db.conn(), &helper).unwrap();

        // Create edge
        let edge = Edge::new("main", "helper", EdgeKind::Calls);
        queries.insert_edge(db.conn(), &edge).unwrap();

        let mut builder = ContextBuilder::new(db.conn(), &mut queries);
        let result = builder.build_for_node("main", "test context").unwrap();

        assert!(result.context.nodes.len() >= 1);
        assert_eq!(result.context.query, "test context");
    }

    #[test]
    fn test_build_for_query() {
        let (db, mut queries) = setup_test_db();

        let func1 = create_test_node("f1", "getUserById", NodeKind::Function);
        let func2 = create_test_node("f2", "getUserList", NodeKind::Function);

        queries.insert_node(db.conn(), &func1).unwrap();
        queries.insert_node(db.conn(), &func2).unwrap();

        let mut builder = ContextBuilder::new(db.conn(), &mut queries);
        let result = builder.build_for_query("getUser").unwrap();

        // Should find both functions
        assert!(result.context.nodes.len() >= 1);
    }

    #[test]
    fn test_build_for_file() {
        let (db, mut queries) = setup_test_db();

        let func1 = create_test_node("f1", "func1", NodeKind::Function);
        let func2 = create_test_node("f2", "func2", NodeKind::Function);

        queries.insert_node(db.conn(), &func1).unwrap();
        queries.insert_node(db.conn(), &func2).unwrap();

        let mut builder = ContextBuilder::new(db.conn(), &mut queries);
        let result = builder.build_for_file("test.rs").unwrap();

        assert_eq!(result.context.nodes.len(), 2);
    }

    #[test]
    fn test_context_truncation() {
        let (db, mut queries) = setup_test_db();

        // Create many nodes
        for i in 0..20 {
            let node = create_test_node(&format!("n{}", i), &format!("func{}", i), NodeKind::Function);
            queries.insert_node(db.conn(), &node).unwrap();
        }

        let options = ContextOptions {
            max_nodes: 5,
            ..Default::default()
        };

        let mut builder = ContextBuilder::with_options(db.conn(), &mut queries, options);
        let result = builder.build_for_query("func").unwrap();

        // Should be truncated
        assert!(result.context.nodes.len() <= 5);
    }
}
