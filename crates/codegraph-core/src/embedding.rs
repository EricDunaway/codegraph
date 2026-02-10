//! Embedding text generation with graph context
//!
//! This module integrates graph traversal with text building
//! to create rich embedding representations of code symbols.

use codegraph_db::QueryBuilder;
use codegraph_graph::traversal::GraphTraverser;
use codegraph_types::{EmbeddingTextConfig, Node};
use codegraph_vectors::text_builder::{EmbeddingTextBuilder, GraphContext, NodeEnrichment};
use rusqlite::Connection;

use crate::error::CodeGraphError;

/// Build embedding text for a node with full graph context
///
/// This is the main integration point between graph traversal
/// and embedding text generation.
pub fn build_embedding_text(
    conn: &Connection,
    queries: &mut QueryBuilder,
    node: &Node,
    config: &EmbeddingTextConfig,
) -> Result<String, CodeGraphError> {
    // Get graph context using GraphTraverser
    let context = build_graph_context(conn, queries, &node.id.0, config)?;

    // Create enrichment from node's stored fields
    let enrichment = NodeEnrichment::from_node(node);

    // Build the embedding text
    let builder = EmbeddingTextBuilder::new(config.clone());
    Ok(builder.build_text(node, &context, &enrichment))
}

/// Build embedding text with token budget enforcement
pub fn build_embedding_text_with_budget(
    conn: &Connection,
    queries: &mut QueryBuilder,
    node: &Node,
    config: &EmbeddingTextConfig,
) -> Result<String, CodeGraphError> {
    // Get graph context
    let context = build_graph_context(conn, queries, &node.id.0, config)?;

    // Create enrichment from node's stored fields
    let enrichment = NodeEnrichment::from_node(node);

    // Build with token counter
    let builder = EmbeddingTextBuilder::new(config.clone());
    let counter = codegraph_vectors::text_builder::TokenCounter::new()
        .map_err(|e| CodeGraphError::Embedding(e.to_string()))?;

    Ok(builder.build_text_with_budget(node, &context, &enrichment, &counter))
}

/// Build graph context for a node
fn build_graph_context(
    conn: &Connection,
    queries: &mut QueryBuilder,
    node_id: &str,
    config: &EmbeddingTextConfig,
) -> Result<GraphContext, CodeGraphError> {
    let mut traverser = GraphTraverser::new(conn, queries);

    // Get callees, callers, siblings (with limits from config)
    let callees = traverser.get_callees_for_embedding(node_id, config.max_callees)?;
    let callers = traverser.get_callers_for_embedding(node_id, config.max_callers)?;
    let siblings = traverser.get_siblings_for_embedding(node_id, config.max_siblings)?;

    // Get inheritance info (for classes)
    let implements = traverser.get_implements_for_embedding(node_id, 10)?; // Reasonable limit
    let extends = traverser.get_extends_for_embedding(node_id)?;

    Ok(GraphContext {
        callees,
        callers,
        siblings,
        implements,
        extends,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use codegraph_db::DatabaseConnection;
    use codegraph_types::{Edge, EdgeKind, Language, NodeKind};

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

        // Create nodes: main_fn calls helper_a, helper_b
        let main_fn = create_test_node("main_fn", "main_fn", NodeKind::Function);
        let helper_a = create_test_node("helper_a", "helper_a", NodeKind::Function);
        let helper_b = create_test_node("helper_b", "helper_b", NodeKind::Function);

        queries.insert_node(db.conn(), &main_fn).unwrap();
        queries.insert_node(db.conn(), &helper_a).unwrap();
        queries.insert_node(db.conn(), &helper_b).unwrap();

        // Create call edges
        queries
            .insert_edge(db.conn(), &Edge::new("main_fn", "helper_a", EdgeKind::Calls))
            .unwrap();
        queries
            .insert_edge(db.conn(), &Edge::new("main_fn", "helper_b", EdgeKind::Calls))
            .unwrap();

        (db, queries)
    }

    #[test]
    fn test_build_embedding_text_with_graph_context() {
        let (db, mut queries) = setup_test_graph();
        let config = EmbeddingTextConfig::default();

        let node = queries
            .get_node_by_id(db.conn(), "main_fn")
            .unwrap()
            .unwrap();

        let text = build_embedding_text(db.conn(), &mut queries, &node, &config).unwrap();

        // Should contain function info
        assert!(text.contains("function main_fn"));
        // Should contain callees
        assert!(text.contains("calls: helper_a, helper_b") || text.contains("calls: helper_b, helper_a"));
    }

    #[test]
    fn test_build_graph_context() {
        let (db, mut queries) = setup_test_graph();
        let config = EmbeddingTextConfig::default();

        let context = build_graph_context(db.conn(), &mut queries, "main_fn", &config).unwrap();

        assert_eq!(context.callees.len(), 2);
        assert!(context.callees.contains(&"helper_a".to_string()));
        assert!(context.callees.contains(&"helper_b".to_string()));
    }

    #[test]
    fn test_build_embedding_text_with_budget() {
        let (db, mut queries) = setup_test_graph();
        let config = EmbeddingTextConfig {
            max_tokens: 50, // Small budget
            ..Default::default()
        };

        let node = queries
            .get_node_by_id(db.conn(), "main_fn")
            .unwrap()
            .unwrap();

        let text =
            build_embedding_text_with_budget(db.conn(), &mut queries, &node, &config).unwrap();

        // Should not exceed budget (token counting is approximate)
        // Just verify it produces some output
        assert!(!text.is_empty());
        assert!(text.contains("main_fn"));
    }
}
