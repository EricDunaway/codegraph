//! Graph traversal algorithms (BFS, DFS)

use crate::error::GraphError;
use codegraph_db::QueryBuilder;
use codegraph_types::{
    Edge, EdgeKind, Node, NodeId, NodeKind, Subgraph, TraversalDirection, TraversalOptions,
};
use rusqlite::Connection;
use std::collections::{HashSet, VecDeque};

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

    fn setup_test_graph() -> (DatabaseConnection, QueryBuilder) {
        let db = DatabaseConnection::open_in_memory().unwrap();
        let mut queries = QueryBuilder::new(db.conn()).unwrap();

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
}
