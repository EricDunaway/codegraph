//! MCP tools implementation

use crate::error::McpError;
use crate::git::{are_hooks_installed, get_git_status, get_last_sync_time, GitStatus};
use crate::protocol::{ContentBlock, ToolCallResult, ToolDefinition};
use codegraph_context::{ContextBuilder, ContextFormat, ContextOptions};
use codegraph_db::QueryBuilder;
use codegraph_graph::{GraphQueryManager, GraphTraverser};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::path::Path;
use std::time::SystemTime;

/// Tool names
pub const TOOL_SEARCH: &str = "codegraph_search";
pub const TOOL_CONTEXT: &str = "codegraph_context";
pub const TOOL_CALLERS: &str = "codegraph_callers";
pub const TOOL_CALLEES: &str = "codegraph_callees";
pub const TOOL_IMPACT: &str = "codegraph_impact";
pub const TOOL_NODE: &str = "codegraph_node";
pub const TOOL_FILE_NODES: &str = "codegraph_file_nodes";
pub const TOOL_STATUS: &str = "codegraph_status";

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
            ToolDefinition {
                name: TOOL_STATUS.to_string(),
                description: "Get index status including dirty files and sync info".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                }),
            },
        ]
    }

    /// Execute a tool call
    pub fn execute(
        conn: &Connection,
        queries: &mut QueryBuilder,
        repo_path: &Path,
        tool_name: &str,
        args: Value,
    ) -> Result<ToolCallResult, McpError> {
        // Get git status for staleness detection
        let git_status = get_git_status(repo_path);

        match tool_name {
            TOOL_SEARCH => Self::tool_search(conn, queries, args, &git_status),
            TOOL_CONTEXT => Self::tool_context(conn, queries, args),
            TOOL_CALLERS => Self::tool_callers(conn, queries, args, &git_status),
            TOOL_CALLEES => Self::tool_callees(conn, queries, args, &git_status),
            TOOL_IMPACT => Self::tool_impact(conn, queries, args, &git_status),
            TOOL_NODE => Self::tool_node(conn, queries, args, &git_status),
            TOOL_FILE_NODES => Self::tool_file_nodes(conn, queries, args, &git_status),
            TOOL_STATUS => Self::tool_status(conn, queries, repo_path, &git_status),
            _ => Err(McpError::ToolNotFound(tool_name.to_string())),
        }
    }

    /// Generate staleness warning if any files are dirty
    fn staleness_warning(file_paths: &[&str], git_status: &GitStatus) -> Option<String> {
        if !git_status.is_git_repo {
            return None;
        }

        let stale_files: Vec<&str> = file_paths
            .iter()
            .filter(|p| git_status.is_dirty(p))
            .copied()
            .collect();

        if stale_files.is_empty() {
            None
        } else if stale_files.len() == 1 {
            Some(format!(
                "\n\n⚠️ **Stale data**: `{}` has uncommitted changes. Results may be outdated.",
                stale_files[0]
            ))
        } else {
            Some(format!(
                "\n\n⚠️ **Stale data**: {} files have uncommitted changes. Results may be outdated.",
                stale_files.len()
            ))
        }
    }

    /// Search for symbols
    fn tool_search(
        conn: &Connection,
        queries: &QueryBuilder,
        args: Value,
        git_status: &GitStatus,
    ) -> Result<ToolCallResult, McpError> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| McpError::InvalidParams("query is required".to_string()))?;

        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;

        let results = queries.search_nodes(conn, query, None, None, limit, 0)?;

        let mut output = format!("Found {} results for '{}':\n\n", results.len(), query);
        let mut file_paths: Vec<&str> = Vec::new();

        for result in &results {
            output.push_str(&format!(
                "- {} `{}` ({}:{}) [id: {}]\n",
                result.node.kind.as_str(),
                result.node.qualified_name,
                result.node.file_path,
                result.node.start_line,
                result.node.id.as_str()
            ));
            file_paths.push(&result.node.file_path);
        }

        if let Some(warning) = Self::staleness_warning(&file_paths, git_status) {
            output.push_str(&warning);
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
        git_status: &GitStatus,
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
        let mut file_paths: Vec<&str> = Vec::new();

        for caller in &callers {
            output.push_str(&format!(
                "- {} `{}` ({}:{}) [id: {}]\n",
                caller.kind.as_str(),
                caller.qualified_name,
                caller.file_path,
                caller.start_line,
                caller.id.as_str()
            ));
            file_paths.push(&caller.file_path);
        }

        if let Some(warning) = Self::staleness_warning(&file_paths, git_status) {
            output.push_str(&warning);
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
        git_status: &GitStatus,
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
        let mut file_paths: Vec<&str> = Vec::new();

        for callee in &callees {
            output.push_str(&format!(
                "- {} `{}` ({}:{}) [id: {}]\n",
                callee.kind.as_str(),
                callee.qualified_name,
                callee.file_path,
                callee.start_line,
                callee.id.as_str()
            ));
            file_paths.push(&callee.file_path);
        }

        if let Some(warning) = Self::staleness_warning(&file_paths, git_status) {
            output.push_str(&warning);
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
        git_status: &GitStatus,
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

        let mut file_paths: Vec<&str> = Vec::new();

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
                file_paths.push(&node.file_path);
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
                file_paths.push(&node.file_path);
            }
            if impact.indirect.len() > 10 {
                output.push_str(&format!("... and {} more\n", impact.indirect.len() - 10));
            }
        }

        if let Some(warning) = Self::staleness_warning(&file_paths, git_status) {
            output.push_str(&warning);
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
        git_status: &GitStatus,
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

        if let Some(ref snippet) = node.code_snippet {
            output.push_str(&format!(
                "\n**Source:**\n```{}\n{}\n```\n",
                node.language.as_str(),
                snippet
            ));
        }

        if let Some(warning) = Self::staleness_warning(&[&node.file_path], git_status) {
            output.push_str(&warning);
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
        git_status: &GitStatus,
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

        if let Some(warning) = Self::staleness_warning(&[file_path], git_status) {
            output.push_str(&warning);
        }

        Ok(ToolCallResult {
            content: vec![ContentBlock::text(output)],
            is_error: false,
        })
    }

    /// Get index status
    fn tool_status(
        conn: &Connection,
        queries: &QueryBuilder,
        repo_path: &Path,
        git_status: &GitStatus,
    ) -> Result<ToolCallResult, McpError> {
        // Get indexed file and node counts via stats
        let stats = queries.get_stats(conn).ok();
        let file_count = stats.as_ref().map(|s| s.file_count).unwrap_or(0);
        let node_count = stats.as_ref().map(|s| s.node_count).unwrap_or(0);

        // Get last sync time
        let last_sync = get_last_sync_time(repo_path);
        let last_sync_str = match last_sync {
            Some(time) => {
                let duration = SystemTime::now()
                    .duration_since(time)
                    .unwrap_or_default();
                let secs = duration.as_secs();
                if secs < 60 {
                    format!("{} seconds ago", secs)
                } else if secs < 3600 {
                    format!("{} minutes ago", secs / 60)
                } else if secs < 86400 {
                    format!("{} hours ago", secs / 3600)
                } else {
                    format!("{} days ago", secs / 86400)
                }
            }
            None => "unknown".to_string(),
        };

        // Check git hooks
        let hooks_installed = are_hooks_installed(repo_path);

        // Build output
        let mut output = String::from("## CodeGraph Index Status\n\n");
        output.push_str(&format!("- **Indexed files:** {}\n", file_count));
        output.push_str(&format!("- **Total nodes:** {}\n", node_count));
        output.push_str(&format!("- **Last sync:** {}\n", last_sync_str));
        output.push_str(&format!(
            "- **Git hooks:** {}\n",
            if hooks_installed { "installed ✓" } else { "not installed" }
        ));

        // Dirty files
        if git_status.is_git_repo {
            let dirty_count = git_status.dirty_count();
            if dirty_count == 0 {
                output.push_str("- **Dirty files:** none (index is up-to-date)\n");
            } else {
                output.push_str(&format!("- **Dirty files:** {} (index may be stale)\n", dirty_count));

                // List dirty files (up to 10)
                output.push_str("\n**Modified/untracked files:**\n");
                let dirty_files: Vec<_> = git_status.dirty_files().into_iter().take(10).collect();
                for file in &dirty_files {
                    output.push_str(&format!("  - {}\n", file));
                }
                if dirty_count > 10 {
                    output.push_str(&format!("  ... and {} more\n", dirty_count - 10));
                }

                output.push_str("\nRun `codegraph sync` to update the index.");
            }
        } else {
            output.push_str("- **Git status:** not a git repository\n");
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
        assert_eq!(defs.len(), 8); // Now 8 tools including status

        let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&TOOL_SEARCH));
        assert!(names.contains(&TOOL_CONTEXT));
        assert!(names.contains(&TOOL_CALLERS));
        assert!(names.contains(&TOOL_STATUS));
    }

    #[test]
    fn test_staleness_warning_no_dirty() {
        let git_status = GitStatus {
            is_git_repo: true,
            ..Default::default()
        };
        let warning = McpTools::staleness_warning(&["src/main.rs"], &git_status);
        assert!(warning.is_none());
    }

    #[test]
    fn test_staleness_warning_with_dirty() {
        let mut git_status = GitStatus {
            is_git_repo: true,
            ..Default::default()
        };
        git_status.modified.insert("src/main.rs".to_string());

        let warning = McpTools::staleness_warning(&["src/main.rs"], &git_status);
        assert!(warning.is_some());
        assert!(warning.unwrap().contains("Stale data"));
    }

    #[test]
    fn test_tool_search() {
        let db = DatabaseConnection::open_in_memory().unwrap();
        let queries = QueryBuilder::new(db.conn()).unwrap();

        let node = create_test_node("n1", "myFunction", NodeKind::Function);
        queries.insert_node(db.conn(), &node).unwrap();

        let git_status = GitStatus::default();
        let args = json!({"query": "myFunction"});
        let result = McpTools::tool_search(db.conn(), &queries, args, &git_status).unwrap();

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

        let git_status = GitStatus::default();
        let args = json!({"node_id": "n1"});
        let result = McpTools::tool_node(db.conn(), &mut queries, args, &git_status).unwrap();

        assert!(!result.is_error);
        if let ContentBlock::Text { text } = &result.content[0] {
            assert!(text.contains("myFunction"));
            assert!(text.contains("function"));
        }
    }
}
