//! High-level graph queries

use crate::error::GraphError;
use codegraph_db::QueryBuilder;
use codegraph_types::{
    EdgeKind, Node, NodeId, NodeKind, Subgraph,
};
use rusqlite::Connection;
use std::collections::{HashSet, VecDeque};

/// Call graph representation
#[derive(Debug, Default)]
pub struct CallGraph {
    /// The subgraph of call relationships
    pub subgraph: Subgraph,
    /// Entry points (nodes with no incoming calls)
    pub entry_points: Vec<NodeId>,
    /// Leaf nodes (nodes with no outgoing calls)
    pub leaf_nodes: Vec<NodeId>,
}

/// Impact radius result
#[derive(Debug, Default)]
pub struct ImpactRadius {
    /// Directly affected nodes (immediate callers)
    pub direct: Vec<Node>,
    /// Indirectly affected nodes (transitive callers)
    pub indirect: Vec<Node>,
    /// Total nodes affected
    pub total_count: usize,
    /// Maximum depth of impact
    pub max_depth: u32,
}

/// Circular dependency information
#[derive(Debug)]
pub struct CircularDependency {
    /// Nodes involved in the cycle
    pub nodes: Vec<NodeId>,
    /// File paths involved
    pub files: Vec<String>,
}

/// Dead code detection result
#[derive(Debug, Default)]
pub struct DeadCodeResult {
    /// Functions/methods never called
    pub unused_functions: Vec<Node>,
    /// Classes/structs never instantiated or referenced
    pub unused_types: Vec<Node>,
    /// Exports never imported
    pub unused_exports: Vec<Node>,
    /// Total dead code count
    pub total_count: usize,
}

/// Metrics for a single node
#[derive(Debug, Default)]
pub struct NodeMetrics {
    /// Number of incoming edges
    pub in_degree: usize,
    /// Number of outgoing edges
    pub out_degree: usize,
    /// Number of direct dependencies
    pub dependencies: usize,
    /// Number of direct dependents
    pub dependents: usize,
    /// Depth in call hierarchy (from entry points)
    pub depth: Option<u32>,
    /// Cyclomatic complexity estimate (based on callees)
    pub complexity: usize,
}

/// High-level graph query manager
pub struct GraphQueryManager<'a> {
    conn: &'a Connection,
    queries: &'a mut QueryBuilder,
}

impl<'a> GraphQueryManager<'a> {
    /// Create a new query manager
    pub fn new(conn: &'a Connection, queries: &'a mut QueryBuilder) -> Self {
        Self { conn, queries }
    }

    /// Build the call graph for a subgraph
    pub fn build_call_graph(&mut self, root_ids: &[&str]) -> Result<CallGraph, GraphError> {
        let mut call_graph = CallGraph::default();
        let mut visited: HashSet<String> = HashSet::new();

        for root_id in root_ids {
            self.collect_call_graph(root_id, &mut call_graph, &mut visited)?;
        }

        // Find entry points (no incoming call edges)
        let incoming_targets: HashSet<_> = call_graph
            .subgraph
            .edges
            .iter()
            .map(|e| e.target.0.clone())
            .collect();

        for id in call_graph.subgraph.nodes.keys() {
            if !incoming_targets.contains(&id.0) {
                call_graph.entry_points.push(id.clone());
            }
        }

        // Find leaf nodes (no outgoing call edges)
        let outgoing_sources: HashSet<_> = call_graph
            .subgraph
            .edges
            .iter()
            .map(|e| e.source.0.clone())
            .collect();

        for id in call_graph.subgraph.nodes.keys() {
            if !outgoing_sources.contains(&id.0) {
                call_graph.leaf_nodes.push(id.clone());
            }
        }

        Ok(call_graph)
    }

    /// Recursive helper for call graph building
    fn collect_call_graph(
        &mut self,
        node_id: &str,
        call_graph: &mut CallGraph,
        visited: &mut HashSet<String>,
    ) -> Result<(), GraphError> {
        if visited.contains(node_id) {
            return Ok(());
        }
        visited.insert(node_id.to_string());

        if let Some(node) = self.queries.get_node_by_id(self.conn, node_id)? {
            call_graph.subgraph.add_node(node);
        }

        let edges = self
            .queries
            .get_outgoing_edges(self.conn, node_id, Some(&[EdgeKind::Calls]))?;

        for edge in edges {
            call_graph.subgraph.add_edge(edge.clone());
            self.collect_call_graph(&edge.target.0, call_graph, visited)?;
        }

        Ok(())
    }

    /// Calculate impact radius - who would be affected if this node changes?
    pub fn get_impact_radius(
        &mut self,
        node_id: &str,
        max_depth: u32,
    ) -> Result<ImpactRadius, GraphError> {
        let mut result = ImpactRadius::default();
        let mut visited: HashSet<String> = HashSet::new();
        let mut queue: VecDeque<(String, u32)> = VecDeque::new();

        visited.insert(node_id.to_string());
        queue.push_back((node_id.to_string(), 0));

        while let Some((current_id, depth)) = queue.pop_front() {
            if depth > max_depth {
                continue;
            }

            result.max_depth = result.max_depth.max(depth);

            // Get all nodes that depend on this one (incoming calls/references)
            let edges = self.queries.get_incoming_edges(
                self.conn,
                &current_id,
                Some(&[EdgeKind::Calls, EdgeKind::References, EdgeKind::Imports]),
            )?;

            for edge in edges {
                let dependent_id = edge.source.0.clone();
                if visited.contains(&dependent_id) {
                    continue;
                }
                visited.insert(dependent_id.clone());

                if let Some(node) = self.queries.get_node_by_id(self.conn, &dependent_id)? {
                    if depth == 0 {
                        result.direct.push(node);
                    } else {
                        result.indirect.push(node);
                    }
                    queue.push_back((dependent_id, depth + 1));
                }
            }
        }

        result.total_count = result.direct.len() + result.indirect.len();
        Ok(result)
    }

    /// Find circular dependencies in the graph
    pub fn find_circular_dependencies(&mut self) -> Result<Vec<CircularDependency>, GraphError> {
        let mut cycles: Vec<CircularDependency> = Vec::new();
        let mut visited: HashSet<String> = HashSet::new();
        let mut rec_stack: HashSet<String> = HashSet::new();
        let mut path: Vec<String> = Vec::new();

        // Get all file nodes
        let files = self.queries.get_nodes_by_kind(self.conn, NodeKind::File)?;

        for file in files {
            if !visited.contains(&file.id.0) {
                self.dfs_cycle_detect(
                    &file.id.0,
                    &mut visited,
                    &mut rec_stack,
                    &mut path,
                    &mut cycles,
                )?;
            }
        }

        Ok(cycles)
    }

    /// DFS helper for cycle detection
    fn dfs_cycle_detect(
        &mut self,
        node_id: &str,
        visited: &mut HashSet<String>,
        rec_stack: &mut HashSet<String>,
        path: &mut Vec<String>,
        cycles: &mut Vec<CircularDependency>,
    ) -> Result<(), GraphError> {
        visited.insert(node_id.to_string());
        rec_stack.insert(node_id.to_string());
        path.push(node_id.to_string());

        let edges = self
            .queries
            .get_outgoing_edges(self.conn, node_id, Some(&[EdgeKind::Imports]))?;

        for edge in edges {
            let neighbor_id = edge.target.0.clone();

            if !visited.contains(&neighbor_id) {
                self.dfs_cycle_detect(&neighbor_id, visited, rec_stack, path, cycles)?;
            } else if rec_stack.contains(&neighbor_id) {
                // Found a cycle! Extract it
                let cycle_start = path.iter().position(|p| p == &neighbor_id);
                if let Some(start) = cycle_start {
                    let cycle_nodes: Vec<NodeId> =
                        path[start..].iter().map(NodeId::new).collect();

                    let files: Vec<String> = cycle_nodes
                        .iter()
                        .filter_map(|id| {
                            self.queries
                                .get_node_by_id(self.conn, id.as_str())
                                .ok()
                                .flatten()
                                .map(|n| n.file_path)
                        })
                        .collect();

                    cycles.push(CircularDependency {
                        nodes: cycle_nodes,
                        files,
                    });
                }
            }
        }

        path.pop();
        rec_stack.remove(node_id);
        Ok(())
    }

    /// Find dead code (unreferenced functions, types, exports)
    pub fn find_dead_code(&mut self) -> Result<DeadCodeResult, GraphError> {
        let mut result = DeadCodeResult::default();

        // Find unused functions (no incoming calls except from tests)
        let functions = self.queries.get_nodes_by_kind(self.conn, NodeKind::Function)?;
        for func in functions {
            let callers = self
                .queries
                .get_incoming_edges(self.conn, &func.id.0, Some(&[EdgeKind::Calls]))?;

            // Exclude main functions and test functions
            if func.name == "main" || func.name.starts_with("test_") {
                continue;
            }

            if callers.is_empty() {
                result.unused_functions.push(func);
            }
        }

        // Find unused types (no instantiation, extension, or reference)
        for kind in [NodeKind::Class, NodeKind::Struct, NodeKind::Interface] {
            let types = self.queries.get_nodes_by_kind(self.conn, kind)?;
            for type_node in types {
                let refs = self.queries.get_incoming_edges(
                    self.conn,
                    &type_node.id.0,
                    Some(&[
                        EdgeKind::Instantiates,
                        EdgeKind::Extends,
                        EdgeKind::Implements,
                        EdgeKind::TypeOf,
                        EdgeKind::References,
                    ]),
                )?;

                if refs.is_empty() {
                    result.unused_types.push(type_node);
                }
            }
        }

        // Find unused exports
        let exports = self.queries.get_nodes_by_kind(self.conn, NodeKind::Export)?;
        for export in exports {
            let imports = self
                .queries
                .get_incoming_edges(self.conn, &export.id.0, Some(&[EdgeKind::Imports]))?;

            if imports.is_empty() {
                result.unused_exports.push(export);
            }
        }

        result.total_count = result.unused_functions.len()
            + result.unused_types.len()
            + result.unused_exports.len();

        Ok(result)
    }

    /// Get metrics for a specific node
    pub fn get_node_metrics(&mut self, node_id: &str) -> Result<NodeMetrics, GraphError> {
        let mut metrics = NodeMetrics::default();

        let incoming = self.queries.get_incoming_edges(self.conn, node_id, None)?;
        let outgoing = self.queries.get_outgoing_edges(self.conn, node_id, None)?;

        metrics.in_degree = incoming.len();
        metrics.out_degree = outgoing.len();

        // Dependencies are outgoing non-contains edges
        metrics.dependencies = outgoing
            .iter()
            .filter(|e| !matches!(e.kind, EdgeKind::Contains))
            .count();

        // Dependents are incoming non-contains edges
        metrics.dependents = incoming
            .iter()
            .filter(|e| !matches!(e.kind, EdgeKind::Contains))
            .count();

        // Complexity estimate based on number of calls made
        let calls = outgoing
            .iter()
            .filter(|e| matches!(e.kind, EdgeKind::Calls))
            .count();
        metrics.complexity = 1 + calls;

        Ok(metrics)
    }

    /// Get type hierarchy (inheritance tree)
    pub fn get_type_hierarchy(&mut self, type_id: &str) -> Result<Subgraph, GraphError> {
        let mut hierarchy = Subgraph::default();
        let mut visited: HashSet<String> = HashSet::new();

        self.collect_type_hierarchy(type_id, &mut hierarchy, &mut visited, true)?;
        self.collect_type_hierarchy(type_id, &mut hierarchy, &mut visited, false)?;

        Ok(hierarchy)
    }

    /// Helper to collect type hierarchy
    fn collect_type_hierarchy(
        &mut self,
        type_id: &str,
        hierarchy: &mut Subgraph,
        visited: &mut HashSet<String>,
        upward: bool,
    ) -> Result<(), GraphError> {
        if visited.contains(type_id) {
            return Ok(());
        }
        visited.insert(type_id.to_string());

        if let Some(node) = self.queries.get_node_by_id(self.conn, type_id)? {
            hierarchy.add_node(node);
        }

        let edge_kinds = &[EdgeKind::Extends, EdgeKind::Implements];

        let edges = if upward {
            self.queries.get_outgoing_edges(self.conn, type_id, Some(edge_kinds))?
        } else {
            self.queries.get_incoming_edges(self.conn, type_id, Some(edge_kinds))?
        };

        for edge in edges {
            hierarchy.add_edge(edge.clone());
            let next_id = if upward { &edge.target.0 } else { &edge.source.0 };
            self.collect_type_hierarchy(next_id, hierarchy, visited, upward)?;
        }

        Ok(())
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
            format!("test.rs::{name}"),
            "test.rs",
            Language::Rust,
            1,
            10,
        )
    }

    fn setup_call_graph() -> (DatabaseConnection, QueryBuilder) {
        let db = DatabaseConnection::open_in_memory().unwrap();
        let queries = QueryBuilder::new(db.conn()).unwrap();

        // Create a call graph: main -> a -> b, main -> c, a -> c
        let main = create_test_node("main", "main", NodeKind::Function);
        let a = create_test_node("a", "funcA", NodeKind::Function);
        let b = create_test_node("b", "funcB", NodeKind::Function);
        let c = create_test_node("c", "funcC", NodeKind::Function);
        let unused = create_test_node("unused", "unusedFunc", NodeKind::Function);

        queries.insert_node(db.conn(), &main).unwrap();
        queries.insert_node(db.conn(), &a).unwrap();
        queries.insert_node(db.conn(), &b).unwrap();
        queries.insert_node(db.conn(), &c).unwrap();
        queries.insert_node(db.conn(), &unused).unwrap();

        queries.insert_edge(db.conn(), &Edge::new("main", "a", EdgeKind::Calls)).unwrap();
        queries.insert_edge(db.conn(), &Edge::new("main", "c", EdgeKind::Calls)).unwrap();
        queries.insert_edge(db.conn(), &Edge::new("a", "b", EdgeKind::Calls)).unwrap();
        queries.insert_edge(db.conn(), &Edge::new("a", "c", EdgeKind::Calls)).unwrap();

        (db, queries)
    }

    #[test]
    fn test_build_call_graph() {
        let (db, mut queries) = setup_call_graph();
        let mut manager = GraphQueryManager::new(db.conn(), &mut queries);

        let call_graph = manager.build_call_graph(&["main"]).unwrap();

        assert_eq!(call_graph.subgraph.node_count(), 4); // main, a, b, c
        assert_eq!(call_graph.entry_points.len(), 1);
        assert_eq!(call_graph.entry_points[0].as_str(), "main");
    }

    #[test]
    fn test_impact_radius() {
        let (db, mut queries) = setup_call_graph();
        let mut manager = GraphQueryManager::new(db.conn(), &mut queries);

        // What's affected if 'c' changes?
        let impact = manager.get_impact_radius("c", 10).unwrap();

        // main and a both call c directly
        assert_eq!(impact.direct.len(), 2);
        assert_eq!(impact.total_count, 2);
    }

    #[test]
    fn test_find_dead_code() {
        let (db, mut queries) = setup_call_graph();
        let mut manager = GraphQueryManager::new(db.conn(), &mut queries);

        let dead_code = manager.find_dead_code().unwrap();

        // 'unused' function is never called
        assert_eq!(dead_code.unused_functions.len(), 1);
        assert_eq!(dead_code.unused_functions[0].name, "unusedFunc");
    }

    #[test]
    fn test_node_metrics() {
        let (db, mut queries) = setup_call_graph();
        let mut manager = GraphQueryManager::new(db.conn(), &mut queries);

        let metrics = manager.get_node_metrics("a").unwrap();

        assert_eq!(metrics.in_degree, 1); // main calls a
        assert_eq!(metrics.out_degree, 2); // a calls b and c
        assert_eq!(metrics.dependents, 1);
        assert_eq!(metrics.dependencies, 2);
    }
}
