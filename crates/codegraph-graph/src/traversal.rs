//! Graph traversal algorithms (BFS, DFS)

use crate::error::GraphError;
use codegraph_db::QueryBuilder;
use codegraph_types::{
    Edge, EdgeKind, Node, NodeId, Subgraph, TraversalDirection, TraversalOptions,
};
use rusqlite::Connection;
use std::collections::{HashMap, HashSet, VecDeque};

/// Result of a graph traversal
#[derive(Debug, Default)]
pub struct TraversalResult {
    /// Subgraph containing visited nodes and edges
    pub subgraph: Subgraph,
    /// Traversal depth reached
    pub max_depth_reached: u32,
    /// Whether traversal was truncated due to limit
    pub truncated: bool,
}

/// Graph traverser with configurable BFS/DFS
pub struct GraphTraverser<'a> {
    conn: &'a Connection,
    queries: &'a mut QueryBuilder,
}

impl<'a> GraphTraverser<'a> {
    /// Create a new traverser
    pub fn new(conn: &'a Connection, queries: &'a mut QueryBuilder) -> Self {
        Self { conn, queries }
    }

    /// Perform BFS traversal from a starting node
    pub fn bfs(
        &mut self,
        start_id: &str,
        options: &TraversalOptions,
    ) -> Result<TraversalResult, GraphError> {
        let mut result = TraversalResult::default();
        let mut visited: HashSet<String> = HashSet::new();
        let mut queue: VecDeque<(String, u32)> = VecDeque::new();

        let max_depth = options.max_depth.unwrap_or(u32::MAX);
        let limit = options.limit.unwrap_or(usize::MAX);

        // Get start node
        let start_node = self
            .queries
            .get_node_by_id(self.conn, start_id)?
            .ok_or_else(|| GraphError::NodeNotFound(start_id.to_string()))?;

        if options.include_start {
            result.subgraph.add_node(start_node.clone());
            result.subgraph.roots.push(start_node.id.clone());
        }

        visited.insert(start_id.to_string());
        queue.push_back((start_id.to_string(), 0));

        while let Some((current_id, depth)) = queue.pop_front() {
            if depth >= max_depth {
                continue;
            }

            if result.subgraph.node_count() >= limit {
                result.truncated = true;
                break;
            }

            result.max_depth_reached = result.max_depth_reached.max(depth);

            // Get edges based on direction
            let edges = self.get_edges(&current_id, options)?;

            for edge in edges {
                let neighbor_id = match options.direction {
                    TraversalDirection::Outgoing => edge.target.0.clone(),
                    TraversalDirection::Incoming => edge.source.0.clone(),
                    TraversalDirection::Both => {
                        if edge.source.0 == current_id {
                            edge.target.0.clone()
                        } else {
                            edge.source.0.clone()
                        }
                    }
                };

                if visited.contains(&neighbor_id) {
                    continue;
                }

                // Get neighbor node and check filters
                if let Some(neighbor) = self.queries.get_node_by_id(self.conn, &neighbor_id)? {
                    if self.should_include_node(&neighbor, options) {
                        visited.insert(neighbor_id.clone());
                        result.subgraph.add_node(neighbor);
                        result.subgraph.add_edge(edge);
                        queue.push_back((neighbor_id, depth + 1));
                    }
                }
            }
        }

        Ok(result)
    }

    /// Perform DFS traversal from a starting node
    pub fn dfs(
        &mut self,
        start_id: &str,
        options: &TraversalOptions,
    ) -> Result<TraversalResult, GraphError> {
        let mut result = TraversalResult::default();
        let mut visited: HashSet<String> = HashSet::new();
        let mut stack: Vec<(String, u32)> = Vec::new();

        let max_depth = options.max_depth.unwrap_or(u32::MAX);
        let limit = options.limit.unwrap_or(usize::MAX);

        // Get start node
        let start_node = self
            .queries
            .get_node_by_id(self.conn, start_id)?
            .ok_or_else(|| GraphError::NodeNotFound(start_id.to_string()))?;

        if options.include_start {
            result.subgraph.add_node(start_node.clone());
            result.subgraph.roots.push(start_node.id.clone());
        }

        visited.insert(start_id.to_string());
        stack.push((start_id.to_string(), 0));

        while let Some((current_id, depth)) = stack.pop() {
            if depth >= max_depth {
                continue;
            }

            if result.subgraph.node_count() >= limit {
                result.truncated = true;
                break;
            }

            result.max_depth_reached = result.max_depth_reached.max(depth);

            let edges = self.get_edges(&current_id, options)?;

            for edge in edges {
                let neighbor_id = match options.direction {
                    TraversalDirection::Outgoing => edge.target.0.clone(),
                    TraversalDirection::Incoming => edge.source.0.clone(),
                    TraversalDirection::Both => {
                        if edge.source.0 == current_id {
                            edge.target.0.clone()
                        } else {
                            edge.source.0.clone()
                        }
                    }
                };

                if visited.contains(&neighbor_id) {
                    continue;
                }

                if let Some(neighbor) = self.queries.get_node_by_id(self.conn, &neighbor_id)? {
                    if self.should_include_node(&neighbor, options) {
                        visited.insert(neighbor_id.clone());
                        result.subgraph.add_node(neighbor);
                        result.subgraph.add_edge(edge);
                        stack.push((neighbor_id, depth + 1));
                    }
                }
            }
        }

        Ok(result)
    }

    /// Get all callers of a node (who calls this?)
    pub fn get_callers(&mut self, node_id: &str) -> Result<Vec<Node>, GraphError> {
        let edges = self.queries.get_incoming_edges(
            self.conn,
            node_id,
            Some(&[EdgeKind::Calls]),
        )?;

        let mut callers = Vec::new();
        for edge in edges {
            if let Some(node) = self.queries.get_node_by_id(self.conn, &edge.source.0)? {
                callers.push(node);
            }
        }

        Ok(callers)
    }

    /// Get all callees of a node (what does this call?)
    pub fn get_callees(&mut self, node_id: &str) -> Result<Vec<Node>, GraphError> {
        let edges = self.queries.get_outgoing_edges(
            self.conn,
            node_id,
            Some(&[EdgeKind::Calls]),
        )?;

        let mut callees = Vec::new();
        for edge in edges {
            if let Some(node) = self.queries.get_node_by_id(self.conn, &edge.target.0)? {
                callees.push(node);
            }
        }

        Ok(callees)
    }

    /// Find a path between two nodes
    pub fn find_path(
        &mut self,
        from_id: &str,
        to_id: &str,
        max_depth: u32,
    ) -> Result<Option<Vec<NodeId>>, GraphError> {
        let mut visited: HashSet<String> = HashSet::new();
        let mut queue: VecDeque<(String, Vec<NodeId>)> = VecDeque::new();

        visited.insert(from_id.to_string());
        queue.push_back((from_id.to_string(), vec![NodeId::new(from_id)]));

        while let Some((current_id, path)) = queue.pop_front() {
            if path.len() > max_depth as usize {
                continue;
            }

            if current_id == to_id {
                return Ok(Some(path));
            }

            let edges = self.queries.get_outgoing_edges(self.conn, &current_id, None)?;

            for edge in edges {
                let neighbor_id = edge.target.0.clone();
                if !visited.contains(&neighbor_id) {
                    visited.insert(neighbor_id.clone());
                    let mut new_path = path.clone();
                    new_path.push(NodeId::new(&neighbor_id));
                    queue.push_back((neighbor_id, new_path));
                }
            }
        }

        Ok(None)
    }

    /// Get edges based on traversal direction
    fn get_edges(
        &self,
        node_id: &str,
        options: &TraversalOptions,
    ) -> Result<Vec<Edge>, GraphError> {
        let edge_kinds = options.edge_kinds.as_deref();

        match options.direction {
            TraversalDirection::Outgoing => {
                Ok(self.queries.get_outgoing_edges(self.conn, node_id, edge_kinds)?)
            }
            TraversalDirection::Incoming => {
                Ok(self.queries.get_incoming_edges(self.conn, node_id, edge_kinds)?)
            }
            TraversalDirection::Both => {
                let mut edges = self.queries.get_outgoing_edges(self.conn, node_id, edge_kinds)?;
                edges.extend(self.queries.get_incoming_edges(self.conn, node_id, edge_kinds)?);
                Ok(edges)
            }
        }
    }

    /// Check if a node should be included based on options
    fn should_include_node(&self, node: &Node, options: &TraversalOptions) -> bool {
        if let Some(ref kinds) = options.node_kinds {
            if !kinds.contains(&node.kind) {
                return false;
            }
        }
        true
    }

    // =========================================================================
    // Embedding Context Methods (G4, G5)
    // Direct relationships only - no transitive, which makes cycles safe
    // =========================================================================

    /// Get callees (what this node calls) for embedding context
    ///
    /// Returns names (not IDs) of directly called functions, limited to `limit`.
    /// G4: Direct relationships only, no transitive traversal.
    /// G5: Direct-only means cycles don't cause infinite loops.
    /// G3: Sorted by decorated first, then call frequency (G6), then alphabetical.
    pub fn get_callees_for_embedding(
        &mut self,
        node_id: &str,
        limit: usize,
    ) -> Result<Vec<String>, GraphError> {
        let edges = self.queries.get_outgoing_edges(
            self.conn,
            node_id,
            Some(&[EdgeKind::Calls]),
        )?;

        // Collect nodes
        let mut nodes = Vec::new();
        for edge in edges {
            if let Some(node) = self.queries.get_node_by_id(self.conn, &edge.target.0)? {
                nodes.push(node);
            }
        }

        // Sort by priority (G3, G6)
        self.sort_nodes_by_priority(&mut nodes)?;

        // Extract names with limit
        Ok(nodes.into_iter().take(limit).map(|n| n.name).collect())
    }

    /// Get callers (who calls this node) for embedding context
    ///
    /// Returns names (not IDs) of functions that directly call this, limited to `limit`.
    /// G4: Direct relationships only, no transitive traversal.
    /// G3: Sorted by decorated first, then call frequency (G6), then alphabetical.
    pub fn get_callers_for_embedding(
        &mut self,
        node_id: &str,
        limit: usize,
    ) -> Result<Vec<String>, GraphError> {
        let edges = self.queries.get_incoming_edges(
            self.conn,
            node_id,
            Some(&[EdgeKind::Calls]),
        )?;

        // Collect nodes
        let mut nodes = Vec::new();
        for edge in edges {
            if let Some(node) = self.queries.get_node_by_id(self.conn, &edge.source.0)? {
                nodes.push(node);
            }
        }

        // Sort by priority (G3, G6)
        self.sort_nodes_by_priority(&mut nodes)?;

        // Extract names with limit
        Ok(nodes.into_iter().take(limit).map(|n| n.name).collect())
    }

    /// Get siblings (nodes in the same container) for embedding context
    ///
    /// Returns names of other functions/methods in the same class/module, limited to `limit`.
    /// Uses Contains edges to find siblings through the parent container.
    pub fn get_siblings_for_embedding(
        &mut self,
        node_id: &str,
        limit: usize,
    ) -> Result<Vec<String>, GraphError> {
        // Find parent container (incoming Contains edge)
        let parent_edges = self.queries.get_incoming_edges(
            self.conn,
            node_id,
            Some(&[EdgeKind::Contains]),
        )?;

        let mut names = Vec::new();

        for parent_edge in parent_edges {
            // Get all children of this parent
            let sibling_edges = self.queries.get_outgoing_edges(
                self.conn,
                &parent_edge.source.0,
                Some(&[EdgeKind::Contains]),
            )?;

            for sibling_edge in sibling_edges {
                // Skip self
                if sibling_edge.target.0 == node_id {
                    continue;
                }

                if names.len() >= limit {
                    break;
                }

                if let Some(node) = self.queries.get_node_by_id(self.conn, &sibling_edge.target.0)? {
                    names.push(node.name);
                }
            }

            if names.len() >= limit {
                break;
            }
        }

        Ok(names)
    }

    /// Get interfaces/traits this node implements for embedding context
    ///
    /// Returns names of implemented interfaces/traits, limited to `limit`.
    pub fn get_implements_for_embedding(
        &mut self,
        node_id: &str,
        limit: usize,
    ) -> Result<Vec<String>, GraphError> {
        let edges = self.queries.get_outgoing_edges(
            self.conn,
            node_id,
            Some(&[EdgeKind::Implements]),
        )?;

        let mut names = Vec::new();
        for edge in edges.into_iter().take(limit) {
            if let Some(node) = self.queries.get_node_by_id(self.conn, &edge.target.0)? {
                names.push(node.name);
            }
        }

        Ok(names)
    }

    /// Get parent class this node extends for embedding context
    ///
    /// Returns the name of the parent class, if any.
    pub fn get_extends_for_embedding(
        &mut self,
        node_id: &str,
    ) -> Result<Option<String>, GraphError> {
        let edges = self.queries.get_outgoing_edges(
            self.conn,
            node_id,
            Some(&[EdgeKind::Extends]),
        )?;

        if let Some(edge) = edges.into_iter().next() {
            if let Some(node) = self.queries.get_node_by_id(self.conn, &edge.target.0)? {
                return Ok(Some(node.name));
            }
        }

        Ok(None)
    }

    /// Sort nodes by priority for embedding context (G3, G6)
    ///
    /// Priority order:
    /// 1. Decorated nodes first
    /// 2. Higher call frequency (counted at query time - G6)
    /// 3. Alphabetical tiebreaker
    fn sort_nodes_by_priority(&self, nodes: &mut [Node]) -> Result<(), GraphError> {
        // Count incoming calls for each node (G6: count at query time)
        let mut call_counts: HashMap<String, usize> = HashMap::new();
        for node in nodes.iter() {
            let count = self.count_incoming_calls(&node.id.0)?;
            call_counts.insert(node.id.0.clone(), count);
        }

        nodes.sort_by(|a, b| {
            // 1. Decorated first
            let a_decorated = !a.decorators.is_empty();
            let b_decorated = !b.decorators.is_empty();
            if a_decorated != b_decorated {
                return b_decorated.cmp(&a_decorated);
            }

            // 2. Higher call frequency (G6)
            let a_freq = call_counts.get(&a.id.0).unwrap_or(&0);
            let b_freq = call_counts.get(&b.id.0).unwrap_or(&0);
            if a_freq != b_freq {
                return b_freq.cmp(a_freq);
            }

            // 3. Alphabetical tiebreaker
            a.name.cmp(&b.name)
        });

        Ok(())
    }

    /// Count incoming calls to a node (for priority sorting - G6)
    fn count_incoming_calls(&self, node_id: &str) -> Result<usize, GraphError> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM edges WHERE target = ?1 AND kind = 'calls'",
            rusqlite::params![node_id],
            |row| row.get(0),
        ).map_err(|e| GraphError::Database(codegraph_db::DbError::Sqlite(e)))?;
        Ok(count as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codegraph_db::DatabaseConnection;
    use codegraph_types::{Language, NodeKind};

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

    fn setup_test_graph() -> (DatabaseConnection, QueryBuilder) {
        let db = DatabaseConnection::open_in_memory().unwrap();
        let queries = QueryBuilder::new(db.conn()).unwrap();

        // Create nodes: A -> B -> C, A -> D
        let node_a = create_test_node("a", "funcA", NodeKind::Function);
        let node_b = create_test_node("b", "funcB", NodeKind::Function);
        let node_c = create_test_node("c", "funcC", NodeKind::Function);
        let node_d = create_test_node("d", "funcD", NodeKind::Function);

        queries.insert_node(db.conn(), &node_a).unwrap();
        queries.insert_node(db.conn(), &node_b).unwrap();
        queries.insert_node(db.conn(), &node_c).unwrap();
        queries.insert_node(db.conn(), &node_d).unwrap();

        // Create edges
        queries.insert_edge(db.conn(), &Edge::new("a", "b", EdgeKind::Calls)).unwrap();
        queries.insert_edge(db.conn(), &Edge::new("b", "c", EdgeKind::Calls)).unwrap();
        queries.insert_edge(db.conn(), &Edge::new("a", "d", EdgeKind::Calls)).unwrap();

        (db, queries)
    }

    #[test]
    fn test_bfs_traversal() {
        let (db, mut queries) = setup_test_graph();
        let mut traverser = GraphTraverser::new(db.conn(), &mut queries);

        let options = TraversalOptions {
            direction: TraversalDirection::Outgoing,
            include_start: true,
            ..Default::default()
        };

        let result = traverser.bfs("a", &options).unwrap();

        // Should find A, B, C, D
        assert_eq!(result.subgraph.node_count(), 4);
        assert!(!result.truncated);
    }

    #[test]
    fn test_bfs_with_depth_limit() {
        let (db, mut queries) = setup_test_graph();
        let mut traverser = GraphTraverser::new(db.conn(), &mut queries);

        let options = TraversalOptions {
            direction: TraversalDirection::Outgoing,
            include_start: true,
            max_depth: Some(1),
            ..Default::default()
        };

        let result = traverser.bfs("a", &options).unwrap();

        // Should find A, B, D (not C which is depth 2)
        assert_eq!(result.subgraph.node_count(), 3);
    }

    #[test]
    fn test_get_callers() {
        let (db, mut queries) = setup_test_graph();
        let mut traverser = GraphTraverser::new(db.conn(), &mut queries);

        let callers = traverser.get_callers("b").unwrap();

        assert_eq!(callers.len(), 1);
        assert_eq!(callers[0].name, "funcA");
    }

    #[test]
    fn test_get_callees() {
        let (db, mut queries) = setup_test_graph();
        let mut traverser = GraphTraverser::new(db.conn(), &mut queries);

        let callees = traverser.get_callees("a").unwrap();

        assert_eq!(callees.len(), 2);
        let names: Vec<_> = callees.iter().map(|n| n.name.as_str()).collect();
        assert!(names.contains(&"funcB"));
        assert!(names.contains(&"funcD"));
    }

    #[test]
    fn test_find_path() {
        let (db, mut queries) = setup_test_graph();
        let mut traverser = GraphTraverser::new(db.conn(), &mut queries);

        let path = traverser.find_path("a", "c", 10).unwrap();

        assert!(path.is_some());
        let path = path.unwrap();
        assert_eq!(path.len(), 3); // A -> B -> C
        assert_eq!(path[0].as_str(), "a");
        assert_eq!(path[2].as_str(), "c");
    }

    #[test]
    fn test_find_path_no_path() {
        let (db, mut queries) = setup_test_graph();
        let mut traverser = GraphTraverser::new(db.conn(), &mut queries);

        // C has no outgoing edges to A
        let path = traverser.find_path("c", "a", 10).unwrap();
        assert!(path.is_none());
    }

    // =========================================================================
    // Embedding Context Tests (Task 15: G4, G5)
    // =========================================================================

    #[test]
    fn test_get_callees_for_embedding() {
        let (db, mut queries) = setup_test_graph();
        let mut traverser = GraphTraverser::new(db.conn(), &mut queries);

        // A calls B and D
        let callees = traverser.get_callees_for_embedding("a", 10).unwrap();

        assert_eq!(callees.len(), 2);
        // Should be names, not IDs
        assert!(callees.contains(&"funcB".to_string()));
        assert!(callees.contains(&"funcD".to_string()));
    }

    #[test]
    fn test_get_callees_for_embedding_respects_limit() {
        let (db, mut queries) = setup_test_graph();
        let mut traverser = GraphTraverser::new(db.conn(), &mut queries);

        // A calls B and D, but limit to 1
        let callees = traverser.get_callees_for_embedding("a", 1).unwrap();

        assert_eq!(callees.len(), 1);
    }

    #[test]
    fn test_get_callers_for_embedding() {
        let (db, mut queries) = setup_test_graph();
        let mut traverser = GraphTraverser::new(db.conn(), &mut queries);

        // B is called by A
        let callers = traverser.get_callers_for_embedding("b", 10).unwrap();

        assert_eq!(callers.len(), 1);
        assert!(callers.contains(&"funcA".to_string()));
    }

    #[test]
    fn test_direct_only_no_transitive() {
        // G4: Direct relationships only, no transitive
        let (db, mut queries) = setup_test_graph();
        let mut traverser = GraphTraverser::new(db.conn(), &mut queries);

        // A -> B -> C chain, but should only get direct callees
        let callees = traverser.get_callees_for_embedding("a", 10).unwrap();

        // Should only get B and D, NOT C (which is transitive)
        assert!(callees.contains(&"funcB".to_string()));
        assert!(callees.contains(&"funcD".to_string()));
        assert!(!callees.contains(&"funcC".to_string()));
    }

    fn setup_cyclic_graph() -> (DatabaseConnection, QueryBuilder) {
        // fn_a -> fn_b -> fn_c -> fn_a (cycle!)
        let db = DatabaseConnection::open_in_memory().unwrap();
        let queries = QueryBuilder::new(db.conn()).unwrap();

        let node_a = create_test_node("fn_a", "fn_a", NodeKind::Function);
        let node_b = create_test_node("fn_b", "fn_b", NodeKind::Function);
        let node_c = create_test_node("fn_c", "fn_c", NodeKind::Function);

        queries.insert_node(db.conn(), &node_a).unwrap();
        queries.insert_node(db.conn(), &node_b).unwrap();
        queries.insert_node(db.conn(), &node_c).unwrap();

        queries.insert_edge(db.conn(), &Edge::new("fn_a", "fn_b", EdgeKind::Calls)).unwrap();
        queries.insert_edge(db.conn(), &Edge::new("fn_b", "fn_c", EdgeKind::Calls)).unwrap();
        queries.insert_edge(db.conn(), &Edge::new("fn_c", "fn_a", EdgeKind::Calls)).unwrap();

        (db, queries)
    }

    #[test]
    fn test_cycle_handling_safe() {
        // G5: Direct-only (G4) means cycles don't cause infinite loops
        let (db, mut queries) = setup_cyclic_graph();
        let mut traverser = GraphTraverser::new(db.conn(), &mut queries);

        // This should NOT hang or panic - direct-only makes it safe
        let callees_a = traverser.get_callees_for_embedding("fn_a", 10).unwrap();
        let callees_b = traverser.get_callees_for_embedding("fn_b", 10).unwrap();
        let callees_c = traverser.get_callees_for_embedding("fn_c", 10).unwrap();

        // Each node only sees its direct callee
        assert_eq!(callees_a.len(), 1);
        assert!(callees_a.contains(&"fn_b".to_string()));
        assert_eq!(callees_b.len(), 1);
        assert!(callees_b.contains(&"fn_c".to_string()));
        assert_eq!(callees_c.len(), 1);
        assert!(callees_c.contains(&"fn_a".to_string())); // Cycle edge, but safe
    }

    fn setup_container_graph() -> (DatabaseConnection, QueryBuilder) {
        // Class MyClass contains method1, method2, method3
        let db = DatabaseConnection::open_in_memory().unwrap();
        let queries = QueryBuilder::new(db.conn()).unwrap();

        let class_node = create_test_node("my_class", "MyClass", NodeKind::Class);
        let method1 = create_test_node("method1", "method1", NodeKind::Method);
        let method2 = create_test_node("method2", "method2", NodeKind::Method);
        let method3 = create_test_node("method3", "method3", NodeKind::Method);

        queries.insert_node(db.conn(), &class_node).unwrap();
        queries.insert_node(db.conn(), &method1).unwrap();
        queries.insert_node(db.conn(), &method2).unwrap();
        queries.insert_node(db.conn(), &method3).unwrap();

        // Class contains all methods
        queries.insert_edge(db.conn(), &Edge::new("my_class", "method1", EdgeKind::Contains)).unwrap();
        queries.insert_edge(db.conn(), &Edge::new("my_class", "method2", EdgeKind::Contains)).unwrap();
        queries.insert_edge(db.conn(), &Edge::new("my_class", "method3", EdgeKind::Contains)).unwrap();

        (db, queries)
    }

    #[test]
    fn test_get_siblings_for_embedding() {
        let (db, mut queries) = setup_container_graph();
        let mut traverser = GraphTraverser::new(db.conn(), &mut queries);

        // method1 siblings are method2 and method3
        let siblings = traverser.get_siblings_for_embedding("method1", 10).unwrap();

        assert_eq!(siblings.len(), 2);
        assert!(siblings.contains(&"method2".to_string()));
        assert!(siblings.contains(&"method3".to_string()));
        // Should not include self
        assert!(!siblings.contains(&"method1".to_string()));
    }

    #[test]
    fn test_get_siblings_respects_limit() {
        let (db, mut queries) = setup_container_graph();
        let mut traverser = GraphTraverser::new(db.conn(), &mut queries);

        // Limit to 1 sibling
        let siblings = traverser.get_siblings_for_embedding("method1", 1).unwrap();

        assert_eq!(siblings.len(), 1);
    }

    fn setup_inheritance_graph() -> (DatabaseConnection, QueryBuilder) {
        // PaymentService extends BaseService, implements IPayment, IRefundable
        let db = DatabaseConnection::open_in_memory().unwrap();
        let queries = QueryBuilder::new(db.conn()).unwrap();

        let payment_service = create_test_node("payment_service", "PaymentService", NodeKind::Class);
        let base_service = create_test_node("base_service", "BaseService", NodeKind::Class);
        let i_payment = create_test_node("i_payment", "IPayment", NodeKind::Interface);
        let i_refundable = create_test_node("i_refundable", "IRefundable", NodeKind::Interface);

        queries.insert_node(db.conn(), &payment_service).unwrap();
        queries.insert_node(db.conn(), &base_service).unwrap();
        queries.insert_node(db.conn(), &i_payment).unwrap();
        queries.insert_node(db.conn(), &i_refundable).unwrap();

        queries.insert_edge(db.conn(), &Edge::new("payment_service", "base_service", EdgeKind::Extends)).unwrap();
        queries.insert_edge(db.conn(), &Edge::new("payment_service", "i_payment", EdgeKind::Implements)).unwrap();
        queries.insert_edge(db.conn(), &Edge::new("payment_service", "i_refundable", EdgeKind::Implements)).unwrap();

        (db, queries)
    }

    #[test]
    fn test_get_implements_for_embedding() {
        let (db, mut queries) = setup_inheritance_graph();
        let mut traverser = GraphTraverser::new(db.conn(), &mut queries);

        let implements = traverser.get_implements_for_embedding("payment_service", 10).unwrap();

        assert_eq!(implements.len(), 2);
        assert!(implements.contains(&"IPayment".to_string()));
        assert!(implements.contains(&"IRefundable".to_string()));
    }

    #[test]
    fn test_get_extends_for_embedding() {
        let (db, mut queries) = setup_inheritance_graph();
        let mut traverser = GraphTraverser::new(db.conn(), &mut queries);

        let extends = traverser.get_extends_for_embedding("payment_service").unwrap();

        assert_eq!(extends, Some("BaseService".to_string()));
    }

    #[test]
    fn test_get_extends_for_embedding_none() {
        let (db, mut queries) = setup_inheritance_graph();
        let mut traverser = GraphTraverser::new(db.conn(), &mut queries);

        // BaseService doesn't extend anything
        let extends = traverser.get_extends_for_embedding("base_service").unwrap();

        assert_eq!(extends, None);
    }

    // =========================================================================
    // Priority Sorting Tests (Task 18: G3, G6)
    // =========================================================================

    fn setup_graph_with_decorated_nodes() -> (DatabaseConnection, QueryBuilder) {
        let db = DatabaseConnection::open_in_memory().unwrap();
        let queries = QueryBuilder::new(db.conn()).unwrap();

        // Caller node
        let caller = create_test_node("caller", "caller", NodeKind::Function);
        queries.insert_node(db.conn(), &caller).unwrap();

        // fn_plain - no decorators
        let mut fn_plain = create_test_node("fn_plain", "fn_plain", NodeKind::Function);
        fn_plain.decorators = vec![];
        queries.insert_node(db.conn(), &fn_plain).unwrap();

        // fn_decorated - has decorators
        let mut fn_decorated = create_test_node("fn_decorated", "fn_decorated", NodeKind::Function);
        fn_decorated.decorators = vec!["@Controller".to_string()];
        queries.insert_node(db.conn(), &fn_decorated).unwrap();

        // caller calls both
        queries.insert_edge(db.conn(), &Edge::new("caller", "fn_plain", EdgeKind::Calls)).unwrap();
        queries.insert_edge(db.conn(), &Edge::new("caller", "fn_decorated", EdgeKind::Calls)).unwrap();

        (db, queries)
    }

    #[test]
    fn test_callees_sorted_decorated_first() {
        let (db, mut queries) = setup_graph_with_decorated_nodes();
        let mut traverser = GraphTraverser::new(db.conn(), &mut queries);

        let callees = traverser.get_callees_for_embedding("caller", 10).unwrap();

        // G3: Decorated nodes should come first
        assert_eq!(callees.len(), 2);
        assert_eq!(callees[0], "fn_decorated", "Decorated should come first");
        assert_eq!(callees[1], "fn_plain", "Plain should come second");
    }

    fn setup_graph_with_call_counts() -> (DatabaseConnection, QueryBuilder) {
        let db = DatabaseConnection::open_in_memory().unwrap();
        let queries = QueryBuilder::new(db.conn()).unwrap();

        // fn_popular - will be called by many
        let fn_popular = create_test_node("fn_popular", "fn_popular", NodeKind::Function);
        queries.insert_node(db.conn(), &fn_popular).unwrap();

        // fn_rare - will be called by few
        let fn_rare = create_test_node("fn_rare", "fn_rare", NodeKind::Function);
        queries.insert_node(db.conn(), &fn_rare).unwrap();

        // Main caller that calls both
        let caller = create_test_node("caller", "caller", NodeKind::Function);
        queries.insert_node(db.conn(), &caller).unwrap();
        queries.insert_edge(db.conn(), &Edge::new("caller", "fn_popular", EdgeKind::Calls)).unwrap();
        queries.insert_edge(db.conn(), &Edge::new("caller", "fn_rare", EdgeKind::Calls)).unwrap();

        // Create many additional callers for fn_popular (G6: count at query time)
        for i in 0..5 {
            let id = format!("extra_caller_{}", i);
            let extra_caller = create_test_node(&id, &id, NodeKind::Function);
            queries.insert_node(db.conn(), &extra_caller).unwrap();
            queries.insert_edge(db.conn(), &Edge::new(id.as_str(), "fn_popular", EdgeKind::Calls)).unwrap();
        }

        // fn_popular now has 6 callers, fn_rare has 1

        (db, queries)
    }

    #[test]
    fn test_callees_sorted_by_call_frequency() {
        let (db, mut queries) = setup_graph_with_call_counts();
        let mut traverser = GraphTraverser::new(db.conn(), &mut queries);

        let callees = traverser.get_callees_for_embedding("caller", 10).unwrap();

        // G3, G6: More frequently called functions come first
        assert_eq!(callees.len(), 2);
        assert_eq!(callees[0], "fn_popular", "Popular (more callers) should come first");
        assert_eq!(callees[1], "fn_rare", "Rare (fewer callers) should come second");
    }

    fn setup_graph_with_same_priority() -> (DatabaseConnection, QueryBuilder) {
        let db = DatabaseConnection::open_in_memory().unwrap();
        let queries = QueryBuilder::new(db.conn()).unwrap();

        // Caller node
        let caller = create_test_node("caller", "caller", NodeKind::Function);
        queries.insert_node(db.conn(), &caller).unwrap();

        // Two functions with same decoration status (none) and same call count (just caller)
        let fn_beta = create_test_node("fn_beta", "fn_beta", NodeKind::Function);
        queries.insert_node(db.conn(), &fn_beta).unwrap();

        let fn_alpha = create_test_node("fn_alpha", "fn_alpha", NodeKind::Function);
        queries.insert_node(db.conn(), &fn_alpha).unwrap();

        // caller calls both (insert beta first to ensure order isn't just insertion order)
        queries.insert_edge(db.conn(), &Edge::new("caller", "fn_beta", EdgeKind::Calls)).unwrap();
        queries.insert_edge(db.conn(), &Edge::new("caller", "fn_alpha", EdgeKind::Calls)).unwrap();

        (db, queries)
    }

    #[test]
    fn test_callees_alphabetical_tiebreaker() {
        let (db, mut queries) = setup_graph_with_same_priority();
        let mut traverser = GraphTraverser::new(db.conn(), &mut queries);

        let callees = traverser.get_callees_for_embedding("caller", 10).unwrap();

        // G3: Same decoration status and frequency -> alphabetical
        assert_eq!(callees.len(), 2);
        assert_eq!(callees[0], "fn_alpha", "Alpha should come before beta");
        assert_eq!(callees[1], "fn_beta", "Beta should come after alpha");
    }
}
