//! MCP tools implementation

use crate::error::McpError;
use crate::protocol::{ContentBlock, ToolCallResult, ToolDefinition};
use codegraph_context::{ContextBuilder, ContextFormat, ContextOptions};
use codegraph_db::QueryBuilder;
use codegraph_graph::{GraphQueryManager, GraphTraverser};
use rusqlite::Connection;
use serde_json::{json, Value};

/// Tool names
pub const TOOL_SEARCH: &str = "codegraph_search";
pub const TOOL_CONTEXT: &str = "codegraph_context";
pub const TOOL_CALLERS: &str = "codegraph_callers";
pub const TOOL_CALLEES: &str = "codegraph_callees";
pub const TOOL_IMPACT: &str = "codegraph_impact";
pub const TOOL_NODE: &str = "codegraph_node";
pub const TOOL_FILE_NODES: &str = "codegraph_file_nodes";

/// MCP tools handler
pub struct McpTools;

impl McpTools {
    /// Get all tool definitions
    pub fn get_definitions() -> Vec<ToolDefinition> {
        vec![
            ToolDefinition {
                name: TOOL_SEARCH.to_string(),
                description: "Search for symbols by name (functions, classes, types)".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "Search query"
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Maximum results (default: 20)"
                        }
                    },
                    "required": ["query"]
                }),
            },
            ToolDefinition {
                name: TOOL_CONTEXT.to_string(),
                description: "Get relevant code context for a task".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "Task or query to get context for"
                        },
                        "max_tokens": {
                            "type": "integer",
                            "description": "Maximum tokens in context (default: 8000)"
                        }
                    },
                    "required": ["query"]
                }),
            },
            ToolDefinition {
                name: TOOL_CALLERS.to_string(),
                description: "Find what calls a function".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "node_id": {
                            "type": "string",
                            "description": "Node ID to find callers for"
                        }
                    },
                    "required": ["node_id"]
                }),
            },
            ToolDefinition {
                name: TOOL_CALLEES.to_string(),
                description: "Find what a function calls".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "node_id": {
                            "type": "string",
                            "description": "Node ID to find callees for"
                        }
                    },
                    "required": ["node_id"]
                }),
            },
            ToolDefinition {
                name: TOOL_IMPACT.to_string(),
                description: "See what's affected by changing a symbol".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "node_id": {
                            "type": "string",
                            "description": "Node ID to analyze impact for"
                        },
                        "max_depth": {
                            "type": "integer",
                            "description": "Maximum depth to traverse (default: 3)"
                        }
                    },
                    "required": ["node_id"]
                }),
            },
            ToolDefinition {
                name: TOOL_NODE.to_string(),
                description: "Get details and source code for a symbol".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "node_id": {
                            "type": "string",
                            "description": "Node ID to get details for"
                        }
                    },
                    "required": ["node_id"]
                }),
            },
            ToolDefinition {
                name: TOOL_FILE_NODES.to_string(),
                description: "Get all symbols in a file".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "file_path": {
                            "type": "string",
                            "description": "File path to get symbols for"
                        }
                    },
                    "required": ["file_path"]
                }),
            },
        ]
    }

    /// Execute a tool call
    pub fn execute(
        conn: &Connection,
        queries: &mut QueryBuilder,
        tool_name: &str,
        args: Value,
    ) -> Result<ToolCallResult, McpError> {
        match tool_name {
            TOOL_SEARCH => Self::tool_search(conn, queries, args),
            TOOL_CONTEXT => Self::tool_context(conn, queries, args),
            TOOL_CALLERS => Self::tool_callers(conn, queries, args),
            TOOL_CALLEES => Self::tool_callees(conn, queries, args),
            TOOL_IMPACT => Self::tool_impact(conn, queries, args),
            TOOL_NODE => Self::tool_node(conn, queries, args),
            TOOL_FILE_NODES => Self::tool_file_nodes(conn, queries, args),
            _ => Err(McpError::ToolNotFound(tool_name.to_string())),
        }
    }

    /// Search for symbols
    fn tool_search(
        conn: &Connection,
        queries: &QueryBuilder,
        args: Value,
    ) -> Result<ToolCallResult, McpError> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| McpError::InvalidParams("query is required".to_string()))?;

        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;

        let results = queries.search_nodes(conn, query, None, None, limit, 0)?;

        let mut output = format!("Found {} results for '{}':\n\n", results.len(), query);
        for result in results {
            output.push_str(&format!(
                "- {} `{}` ({}:{}) [id: {}]\n",
                result.node.kind.as_str(),
                result.node.qualified_name,
                result.node.file_path,
                result.node.start_line,
                result.node.id.as_str()
            ));
        }

        Ok(ToolCallResult {
            content: vec![ContentBlock::text(output)],
            is_error: false,
        })
    }

    /// Get context for a query
    fn tool_context(
        conn: &Connection,
        queries: &mut QueryBuilder,
        args: Value,
    ) -> Result<ToolCallResult, McpError> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| McpError::InvalidParams("query is required".to_string()))?;

        let max_tokens = args.get("max_tokens").and_then(|v| v.as_u64()).unwrap_or(8000) as usize;

        let options = ContextOptions {
            max_tokens,
            format: ContextFormat::Markdown,
            include_source: true,
            ..Default::default()
        };

        let mut builder = ContextBuilder::with_options(conn, queries, options);
        let result = builder.build_for_query(query)?;

        Ok(ToolCallResult {
            content: vec![ContentBlock::text(result.output)],
            is_error: false,
        })
    }

    /// Find callers of a node
    fn tool_callers(
        conn: &Connection,
        queries: &mut QueryBuilder,
        args: Value,
    ) -> Result<ToolCallResult, McpError> {
        let node_id = args
            .get("node_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| McpError::InvalidParams("node_id is required".to_string()))?;

        let mut traverser = GraphTraverser::new(conn, queries);
        let callers = traverser.get_callers(node_id)?;

        if callers.is_empty() {
            return Ok(ToolCallResult {
                content: vec![ContentBlock::text(format!(
                    "No callers found for '{}'",
                    node_id
                ))],
                is_error: false,
            });
        }

        let mut output = format!("Callers of '{}':\n\n", node_id);
        for caller in callers {
            output.push_str(&format!(
                "- {} `{}` ({}:{}) [id: {}]\n",
                caller.kind.as_str(),
                caller.qualified_name,
                caller.file_path,
                caller.start_line,
                caller.id.as_str()
            ));
        }

        Ok(ToolCallResult {
            content: vec![ContentBlock::text(output)],
            is_error: false,
        })
    }

    /// Find callees of a node
    fn tool_callees(
        conn: &Connection,
        queries: &mut QueryBuilder,
        args: Value,
    ) -> Result<ToolCallResult, McpError> {
        let node_id = args
            .get("node_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| McpError::InvalidParams("node_id is required".to_string()))?;

        let mut traverser = GraphTraverser::new(conn, queries);
        let callees = traverser.get_callees(node_id)?;

        if callees.is_empty() {
            return Ok(ToolCallResult {
                content: vec![ContentBlock::text(format!(
                    "No callees found for '{}'",
                    node_id
                ))],
                is_error: false,
            });
        }

        let mut output = format!("Callees of '{}':\n\n", node_id);
        for callee in callees {
            output.push_str(&format!(
                "- {} `{}` ({}:{}) [id: {}]\n",
                callee.kind.as_str(),
                callee.qualified_name,
                callee.file_path,
                callee.start_line,
                callee.id.as_str()
            ));
        }

        Ok(ToolCallResult {
            content: vec![ContentBlock::text(output)],
            is_error: false,
        })
    }

    /// Get impact radius
    fn tool_impact(
        conn: &Connection,
        queries: &mut QueryBuilder,
        args: Value,
    ) -> Result<ToolCallResult, McpError> {
        let node_id = args
            .get("node_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| McpError::InvalidParams("node_id is required".to_string()))?;

        let max_depth = args.get("max_depth").and_then(|v| v.as_u64()).unwrap_or(3) as u32;

        let mut manager = GraphQueryManager::new(conn, queries);
        let impact = manager.get_impact_radius(node_id, max_depth)?;

        let mut output = format!(
            "Impact analysis for '{}':\n\n**Total affected:** {} nodes\n**Max depth:** {}\n\n",
            node_id, impact.total_count, impact.max_depth
        );

        if !impact.direct.is_empty() {
            output.push_str("**Direct dependents:**\n");
            for node in &impact.direct {
                output.push_str(&format!(
                    "- {} `{}` ({}:{}) [id: {}]\n",
                    node.kind.as_str(),
                    node.qualified_name,
                    node.file_path,
                    node.start_line,
                    node.id.as_str()
                ));
            }
        }

        if !impact.indirect.is_empty() {
            output.push_str("\n**Indirect dependents:**\n");
            for node in impact.indirect.iter().take(10) {
                output.push_str(&format!(
                    "- {} `{}` ({}:{}) [id: {}]\n",
                    node.kind.as_str(),
                    node.qualified_name,
                    node.file_path,
                    node.start_line,
                    node.id.as_str()
                ));
            }
            if impact.indirect.len() > 10 {
                output.push_str(&format!("... and {} more\n", impact.indirect.len() - 10));
            }
        }

        Ok(ToolCallResult {
            content: vec![ContentBlock::text(output)],
            is_error: false,
        })
    }

    /// Get node details
    fn tool_node(
        conn: &Connection,
        queries: &mut QueryBuilder,
        args: Value,
    ) -> Result<ToolCallResult, McpError> {
        let node_id = args
            .get("node_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| McpError::InvalidParams("node_id is required".to_string()))?;

        let node = queries
            .get_node_by_id(conn, node_id)?
            .ok_or_else(|| McpError::NodeNotFound(node_id.to_string()))?;

        let mut output = format!(
            "## {} `{}`\n\n",
            node.kind.as_str(),
            node.qualified_name
        );

        output.push_str(&format!("- **File:** {}:{}-{}\n", node.file_path, node.start_line, node.end_line));
        output.push_str(&format!("- **Language:** {}\n", node.language.as_str()));

        if let Some(ref sig) = node.signature {
            output.push_str(&format!("- **Signature:** `{}`\n", sig));
        }

        if let Some(ref doc) = node.docstring {
            output.push_str(&format!("\n**Documentation:**\n> {}\n", doc.replace('\n', "\n> ")));
        }

        Ok(ToolCallResult {
            content: vec![ContentBlock::text(output)],
            is_error: false,
        })
    }

    /// Get all nodes in a file
    fn tool_file_nodes(
        conn: &Connection,
        queries: &QueryBuilder,
        args: Value,
    ) -> Result<ToolCallResult, McpError> {
        let file_path = args
            .get("file_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| McpError::InvalidParams("file_path is required".to_string()))?;

        let nodes = queries.get_nodes_by_file(conn, file_path)?;

        if nodes.is_empty() {
            return Ok(ToolCallResult {
                content: vec![ContentBlock::text(format!(
                    "No nodes found in '{}'",
                    file_path
                ))],
                is_error: false,
            });
        }

        let mut output = format!("Symbols in '{}':\n\n", file_path);
        for node in nodes {
            output.push_str(&format!(
                "- {} `{}` (line {}) [id: {}]\n",
                node.kind.as_str(),
                node.name,
                node.start_line,
                node.id.as_str()
            ));
        }

        Ok(ToolCallResult {
            content: vec![ContentBlock::text(output)],
            is_error: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codegraph_db::DatabaseConnection;
    use codegraph_types::{Language, Node, NodeKind};

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

    #[test]
    fn test_get_definitions() {
        let defs = McpTools::get_definitions();
        assert_eq!(defs.len(), 7);

        let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&TOOL_SEARCH));
        assert!(names.contains(&TOOL_CONTEXT));
        assert!(names.contains(&TOOL_CALLERS));
    }

    #[test]
    fn test_tool_search() {
        let db = DatabaseConnection::open_in_memory().unwrap();
        let queries = QueryBuilder::new(db.conn()).unwrap();

        let node = create_test_node("n1", "myFunction", NodeKind::Function);
        queries.insert_node(db.conn(), &node).unwrap();

        let args = json!({"query": "myFunction"});
        let result = McpTools::tool_search(db.conn(), &queries, args).unwrap();

        assert!(!result.is_error);
        if let ContentBlock::Text { text } = &result.content[0] {
            assert!(text.contains("myFunction"));
        }
    }

    #[test]
    fn test_tool_node() {
        let db = DatabaseConnection::open_in_memory().unwrap();
        let mut queries = QueryBuilder::new(db.conn()).unwrap();

        let node = create_test_node("n1", "myFunction", NodeKind::Function);
        queries.insert_node(db.conn(), &node).unwrap();

        let args = json!({"node_id": "n1"});
        let result = McpTools::tool_node(db.conn(), &mut queries, args).unwrap();

        assert!(!result.is_error);
        if let ContentBlock::Text { text } = &result.content[0] {
            assert!(text.contains("myFunction"));
            assert!(text.contains("function"));
        }
    }

    #[test]
    fn test_tool_not_found() {
        let db = DatabaseConnection::open_in_memory().unwrap();
        let mut queries = QueryBuilder::new(db.conn()).unwrap();

        let result = McpTools::execute(db.conn(), &mut queries, "invalid_tool", json!({}));
        assert!(matches!(result, Err(McpError::ToolNotFound(_))));
    }
}
