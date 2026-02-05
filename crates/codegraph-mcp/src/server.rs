//! MCP server implementation

use crate::error::McpError;
use crate::protocol::{error_codes, JsonRpcRequest, JsonRpcResponse, ToolCallParams};
use crate::tools::McpTools;
use codegraph_db::{DatabaseConnection, QueryBuilder};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

/// MCP server state
pub struct McpServer {
    /// Database connection
    db: DatabaseConnection,
    /// Query builder
    queries: QueryBuilder,
    /// Repository path (for git status checks)
    repo_path: PathBuf,
    /// Whether server is initialized
    initialized: bool,
}

impl McpServer {
    /// Create a new MCP server
    pub fn new(db_path: &str) -> Result<Self, McpError> {
        let db_path = Path::new(db_path);
        let db = DatabaseConnection::open(db_path)?;
        let queries = QueryBuilder::new(db.conn())?;

        // Derive repo path from db path (db is at .codegraph/codegraph.db)
        let repo_path = db_path
            .parent() // .codegraph/
            .and_then(|p| p.parent()) // repo root
            .unwrap_or(Path::new("."))
            .to_path_buf();

        Ok(Self {
            db,
            queries,
            repo_path,
            initialized: false,
        })
    }

    /// Create with in-memory database (for testing)
    pub fn new_in_memory() -> Result<Self, McpError> {
        let db = DatabaseConnection::open_in_memory()?;
        let queries = QueryBuilder::new(db.conn())?;

        Ok(Self {
            db,
            queries,
            repo_path: PathBuf::from("."),
            initialized: false,
        })
    }

    /// Run the server on stdio
    pub fn run(&mut self) -> Result<(), McpError> {
        let stdin = std::io::stdin();
        let stdout = std::io::stdout();
        let mut reader = BufReader::new(stdin.lock());
        let mut writer = stdout.lock();

        log::info!("MCP server started");

        let mut line = String::new();
        while reader.read_line(&mut line)? > 0 {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                line.clear();
                continue;
            }

            // Only send response if this is a request (not a notification)
            if let Some(response) = self.handle_message(trimmed) {
                let response_str = serde_json::to_string(&response)?;
                writeln!(writer, "{}", response_str)?;
                writer.flush()?;
            }

            line.clear();
        }

        log::info!("MCP server stopped");
        Ok(())
    }

    /// Handle a single JSON-RPC message
    /// Returns None for notifications (no response needed)
    pub fn handle_message(&mut self, message: &str) -> Option<JsonRpcResponse> {
        // Parse request
        let request: JsonRpcRequest = match serde_json::from_str(message) {
            Ok(r) => r,
            Err(e) => {
                return Some(JsonRpcResponse::error(
                    None,
                    error_codes::PARSE_ERROR,
                    format!("Parse error: {}", e),
                ));
            }
        };

        // Handle request
        self.handle_request(request)
    }

    /// Handle a parsed request
    /// Returns None for notifications (messages without an id)
    fn handle_request(&mut self, request: JsonRpcRequest) -> Option<JsonRpcResponse> {
        let id = request.id.clone();
        let is_notification = id.is_none();

        // Handle notifications (no response needed)
        if is_notification {
            match request.method.as_str() {
                "notifications/initialized" => {
                    log::info!("Client initialized");
                }
                "notifications/cancelled" => {
                    log::info!("Request cancelled");
                }
                _ => {
                    log::debug!("Unknown notification: {}", request.method);
                }
            }
            return None;
        }

        // Handle requests (response required)
        Some(match request.method.as_str() {
            "initialize" => self.handle_initialize(id, request.params),
            "tools/list" => self.handle_tools_list(id),
            "tools/call" => self.handle_tools_call(id, request.params),
            "shutdown" => self.handle_shutdown(id),
            _ => JsonRpcResponse::error(
                id,
                error_codes::METHOD_NOT_FOUND,
                format!("Method not found: {}", request.method),
            ),
        })
    }

    /// Handle initialize request
    fn handle_initialize(&mut self, id: Option<Value>, _params: Value) -> JsonRpcResponse {
        self.initialized = true;

        let result = json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "tools": {}
            },
            "serverInfo": {
                "name": "codegraph",
                "version": env!("CARGO_PKG_VERSION")
            }
        });

        JsonRpcResponse::success(id, result)
    }

    /// Handle tools/list request
    fn handle_tools_list(&self, id: Option<Value>) -> JsonRpcResponse {
        let tools = McpTools::get_definitions();

        let result = json!({
            "tools": tools
        });

        JsonRpcResponse::success(id, result)
    }

    /// Handle tools/call request
    fn handle_tools_call(&mut self, id: Option<Value>, params: Value) -> JsonRpcResponse {
        // Parse tool call params
        let call_params: ToolCallParams = match serde_json::from_value(params) {
            Ok(p) => p,
            Err(e) => {
                return JsonRpcResponse::error(
                    id,
                    error_codes::INVALID_PARAMS,
                    format!("Invalid params: {}", e),
                );
            }
        };

        // Execute tool
        match McpTools::execute(
            self.db.conn(),
            &mut self.queries,
            &self.repo_path,
            &call_params.name,
            call_params.arguments,
        ) {
            Ok(result) => JsonRpcResponse::success(id, serde_json::to_value(result).unwrap()),
            Err(e) => JsonRpcResponse::error(id, error_codes::INTERNAL_ERROR, e.to_string()),
        }
    }

    /// Handle shutdown request
    fn handle_shutdown(&mut self, id: Option<Value>) -> JsonRpcResponse {
        self.initialized = false;
        JsonRpcResponse::success(id, Value::Null)
    }

    /// Check if server is initialized
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// Get database connection
    pub fn conn(&self) -> &rusqlite::Connection {
        self.db.conn()
    }

    /// Get mutable queries
    pub fn queries_mut(&mut self) -> &mut QueryBuilder {
        &mut self.queries
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_server_creation() {
        let server = McpServer::new_in_memory().unwrap();
        assert!(!server.is_initialized());
    }

    #[test]
    fn test_handle_initialize() {
        let mut server = McpServer::new_in_memory().unwrap();

        let response = server
            .handle_message(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#)
            .expect("should return response for request");

        assert!(response.error.is_none());
        assert!(response.result.is_some());
        assert!(server.is_initialized());
    }

    #[test]
    fn test_handle_tools_list() {
        let mut server = McpServer::new_in_memory().unwrap();

        let response = server
            .handle_message(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#)
            .expect("should return response for request");

        assert!(response.error.is_none());
        let result = response.result.unwrap();
        assert!(result.get("tools").is_some());
    }

    #[test]
    fn test_handle_unknown_method() {
        let mut server = McpServer::new_in_memory().unwrap();

        let response = server
            .handle_message(r#"{"jsonrpc":"2.0","id":1,"method":"unknown","params":{}}"#)
            .expect("should return response for request");

        assert!(response.error.is_some());
        assert_eq!(response.error.unwrap().code, error_codes::METHOD_NOT_FOUND);
    }

    #[test]
    fn test_handle_parse_error() {
        let mut server = McpServer::new_in_memory().unwrap();

        let response = server
            .handle_message("not valid json")
            .expect("should return response for parse error");

        assert!(response.error.is_some());
        assert_eq!(response.error.unwrap().code, error_codes::PARSE_ERROR);
    }

    #[test]
    fn test_handle_notification() {
        let mut server = McpServer::new_in_memory().unwrap();

        // Notifications (no id) should return None
        let response =
            server.handle_message(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);

        assert!(response.is_none());
    }

    #[test]
    fn test_handle_shutdown() {
        let mut server = McpServer::new_in_memory().unwrap();

        // Initialize first
        server.handle_message(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#);
        assert!(server.is_initialized());

        // Shutdown
        server.handle_message(r#"{"jsonrpc":"2.0","id":2,"method":"shutdown","params":{}}"#);
        assert!(!server.is_initialized());
    }
}
