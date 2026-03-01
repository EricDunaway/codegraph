//! Async LSP client with concurrent request support (I5)
//!
//! This module provides a fully async LSP client that supports multiple
//! concurrent in-flight requests to a single LSP server.

use crate::error::LspError;
use lsp_types::{
    GotoDefinitionParams, GotoDefinitionResponse, Hover, HoverParams, InitializeParams,
    InitializeResult, InitializedParams, Position, TextDocumentIdentifier,
    TextDocumentPositionParams, Uri, WorkDoneProgressParams, WorkspaceFolder,
};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::{oneshot, Mutex};
use tokio::task::AbortHandle;
use url::Url;

/// Default timeout for LSP operations
const DEFAULT_TIMEOUT_SECS: u64 = 30;

/// Timeout for graceful shutdown before force kill
const SHUTDOWN_TIMEOUT_SECS: u64 = 5;

/// Map of pending response channels keyed by request ID
type PendingMap = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, LspError>>>>>;

/// Async LSP client supporting concurrent requests (I5)
///
/// Uses a background task to read responses and dispatch them to waiting
/// requests by ID, enabling multiple concurrent in-flight requests.
///
/// This client is designed to be wrapped in Arc for shared access across
/// concurrent tasks without external locking.
pub struct LspClient {
    /// Child process handle
    process: Arc<Mutex<Child>>,
    /// Stdin for sending requests (protected for concurrent writes)
    stdin: Arc<Mutex<ChildStdin>>,
    /// Request ID counter
    next_id: AtomicU64,
    /// Pending response channels by request ID
    pending: PendingMap,
    /// Whether server is initialized
    initialized: Arc<Mutex<bool>>,
    /// Timeout for operations
    timeout: Duration,
    /// Abort handle to stop the reader task on shutdown
    reader_abort: Option<AbortHandle>,
}

impl LspClient {
    /// Spawn a new LSP server process and connect
    pub async fn spawn(command: &str, args: &[String]) -> Result<Self, LspError> {
        Self::spawn_with_timeout(command, args, Duration::from_secs(DEFAULT_TIMEOUT_SECS)).await
    }

    /// Spawn a new LSP server process with custom timeout
    pub async fn spawn_with_timeout(
        command: &str,
        args: &[String],
        timeout: Duration,
    ) -> Result<Self, LspError> {
        let mut process = Command::new(command)
            .args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| LspError::StartFailed(format!("{}: {}", command, e)))?;

        let stdin = process
            .stdin
            .take()
            .ok_or_else(|| LspError::StartFailed("Failed to get stdin".to_string()))?;
        let stdout = process
            .stdout
            .take()
            .ok_or_else(|| LspError::StartFailed("Failed to get stdout".to_string()))?;

        let pending: PendingMap =
            Arc::new(Mutex::new(HashMap::new()));

        // Spawn background task to read and dispatch responses
        let reader_abort = {
            let pending = Arc::clone(&pending);
            let handle = tokio::spawn(async move {
                Self::response_reader_task(stdout, pending).await;
            });
            Some(handle.abort_handle())
        };

        Ok(Self {
            process: Arc::new(Mutex::new(process)),
            stdin: Arc::new(Mutex::new(stdin)),
            next_id: AtomicU64::new(1),
            pending,
            initialized: Arc::new(Mutex::new(false)),
            timeout,
            reader_abort,
        })
    }

    /// Background task that reads responses and dispatches to waiting requests
    async fn response_reader_task(
        stdout: ChildStdout,
        pending: PendingMap,
    ) {
        let mut reader = BufReader::new(stdout);

        loop {
            match Self::read_one_message(&mut reader).await {
                Ok(Some(message)) => {
                    // Check if this is a response (has id)
                    if let Some(id) = message.get("id").and_then(|id| id.as_u64()) {
                        // Find and notify the waiting request
                        let sender = {
                            let mut pending = pending.lock().await;
                            pending.remove(&id)
                        };

                        if let Some(sender) = sender {
                            // Check for error response
                            let result = if let Some(error) = message.get("error") {
                                let code = error.get("code").and_then(|c| c.as_i64()).unwrap_or(-1) as i32;
                                let msg = error
                                    .get("message")
                                    .and_then(|m| m.as_str())
                                    .unwrap_or("Unknown error")
                                    .to_string();
                                Err(LspError::ServerError { code, message: msg })
                            } else {
                                Ok(message)
                            };
                            let _ = sender.send(result);
                        }
                    }
                    // Notifications (no id) are ignored
                }
                Ok(None) => {
                    // EOF - server closed connection
                    log::debug!("LSP server closed stdout");
                    break;
                }
                Err(e) => {
                    log::error!("Error reading LSP response: {}", e);
                    // Notify all pending requests of the error
                    let mut pending = pending.lock().await;
                    for (_, sender) in pending.drain() {
                        let _ = sender.send(Err(LspError::InvalidResponse(
                            "Connection lost".to_string(),
                        )));
                    }
                    break;
                }
            }
        }
    }

    /// Read one LSP message from the reader
    async fn read_one_message(
        reader: &mut BufReader<ChildStdout>,
    ) -> Result<Option<Value>, LspError> {
        // Read headers
        let mut content_length: Option<usize> = None;
        let mut line = String::new();

        loop {
            line.clear();
            let bytes_read = reader.read_line(&mut line).await?;

            if bytes_read == 0 {
                return Ok(None); // EOF
            }

            if line == "\r\n" {
                break; // End of headers
            }

            if line.to_lowercase().starts_with("content-length:") {
                if let Some(len_str) = line.split(':').nth(1) {
                    content_length = len_str.trim().parse().ok();
                }
            }
        }

        let content_length = content_length
            .ok_or_else(|| LspError::InvalidResponse("Missing Content-Length".to_string()))?;

        // Sanity check (10MB max - LSP responses are typically < 100KB)
        if content_length > 10 * 1024 * 1024 {
            return Err(LspError::InvalidResponse(format!(
                "Content-Length too large: {} (max 10MB)",
                content_length
            )));
        }

        // Read content
        let mut content = vec![0u8; content_length];
        reader.read_exact(&mut content).await?;

        let message: Value = serde_json::from_slice(&content)?;
        Ok(Some(message))
    }

    /// Initialize the language server
    #[allow(deprecated)]
    pub async fn initialize(&self, root_uri: &Url) -> Result<InitializeResult, LspError> {
        let uri: Uri = root_uri.as_str().parse().map_err(|e| {
            LspError::InvalidResponse(format!("Invalid URI: {}", e))
        })?;

        let workspace_folder = WorkspaceFolder {
            uri: uri.clone(),
            name: root_uri
                .path_segments()
                .and_then(|mut s| s.next_back())
                .unwrap_or("workspace")
                .to_string(),
        };

        let params = InitializeParams {
            root_uri: Some(uri),
            workspace_folders: Some(vec![workspace_folder]),
            capabilities: lsp_types::ClientCapabilities::default(),
            ..Default::default()
        };

        let result: InitializeResult = self.send_request("initialize", params).await?;

        // Send initialized notification
        self.send_notification("initialized", InitializedParams {}).await?;
        *self.initialized.lock().await = true;

        Ok(result)
    }

    /// Send hover request (can be called concurrently)
    pub async fn hover(
        &self,
        file: &Path,
        line: u32,
        column: u32,
    ) -> Result<Option<Hover>, LspError> {
        if !*self.initialized.lock().await {
            return Err(LspError::NotReady("Server not initialized".to_string()));
        }

        let url = Url::from_file_path(file)
            .map_err(|_| LspError::FileNotFound(file.display().to_string()))?;
        let uri: Uri = url.as_str().parse().map_err(|e| {
            LspError::InvalidResponse(format!("Invalid URI: {}", e))
        })?;

        let params = HoverParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                position: Position::new(line, column),
            },
            work_done_progress_params: WorkDoneProgressParams::default(),
        };

        self.send_request("textDocument/hover", params).await
    }

    /// Send go-to-definition request (can be called concurrently)
    pub async fn definition(
        &self,
        file: &Path,
        line: u32,
        column: u32,
    ) -> Result<Option<GotoDefinitionResponse>, LspError> {
        if !*self.initialized.lock().await {
            return Err(LspError::NotReady("Server not initialized".to_string()));
        }

        let url = Url::from_file_path(file)
            .map_err(|_| LspError::FileNotFound(file.display().to_string()))?;
        let uri: Uri = url.as_str().parse().map_err(|e| {
            LspError::InvalidResponse(format!("Invalid URI: {}", e))
        })?;

        let params = GotoDefinitionParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                position: Position::new(line, column),
            },
            work_done_progress_params: WorkDoneProgressParams::default(),
            partial_result_params: Default::default(),
        };

        self.send_request("textDocument/definition", params).await
    }

    /// Shutdown the server gracefully
    pub async fn shutdown(&self) -> Result<(), LspError> {
        // Abort the reader task first
        if let Some(ref abort) = self.reader_abort {
            abort.abort();
        }

        // Drain all pending requests - notify them that server is shutting down
        {
            let mut pending = self.pending.lock().await;
            for (_, sender) in pending.drain() {
                let _ = sender.send(Err(LspError::NotReady("Server shutting down".to_string())));
            }
        }

        // Send shutdown request
        let result: Result<Option<()>, LspError> = tokio::time::timeout(
            Duration::from_secs(SHUTDOWN_TIMEOUT_SECS),
            self.send_request("shutdown", ()),
        )
        .await
        .map_err(|_| LspError::Timeout("Shutdown request timed out".to_string()))?;

        if let Err(e) = result {
            log::warn!("LSP shutdown request failed: {}", e);
        }

        // Send exit notification
        let _ = self.send_notification("exit", ()).await;

        // Wait for process to exit
        let mut process = self.process.lock().await;
        match tokio::time::timeout(
            Duration::from_secs(SHUTDOWN_TIMEOUT_SECS),
            process.wait(),
        )
        .await
        {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => Err(LspError::Io(e)),
            Err(_) => {
                // Timeout - force kill
                log::warn!("LSP process did not exit, force killing");
                process.kill().await.ok();
                Ok(())
            }
        }
    }

    /// Check if server is initialized
    pub async fn is_initialized(&self) -> bool {
        *self.initialized.lock().await
    }

    /// Set operation timeout
    pub fn set_timeout(&mut self, timeout: Duration) {
        self.timeout = timeout;
    }

    /// Send a JSON-RPC request and wait for response with timeout
    async fn send_request<P: Serialize, R: DeserializeOwned>(
        &self,
        method: &str,
        params: P,
    ) -> Result<R, LspError> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);

        // Create channel for response
        let (tx, rx) = oneshot::channel();

        // Register pending request
        {
            let mut pending = self.pending.lock().await;
            pending.insert(id, tx);
        }

        // Send request
        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });

        if let Err(e) = self.send_message(&request).await {
            // Remove from pending on send failure
            self.pending.lock().await.remove(&id);
            return Err(e);
        }

        // Wait for response with timeout
        let response = match tokio::time::timeout(self.timeout, rx).await {
            Ok(Ok(result)) => result?,
            Ok(Err(_)) => {
                // Channel was closed (sender dropped)
                return Err(LspError::InvalidResponse("Response channel closed".to_string()));
            }
            Err(_) => {
                // Timeout - remove from pending synchronously to avoid race conditions
                // Note: If response arrives after this, the reader task handles it safely
                // by checking if sender exists before sending (line 121: `if let Some(sender)`)
                self.pending.lock().await.remove(&id);
                return Err(LspError::Timeout(format!(
                    "Request {} timed out after {:?}",
                    method, self.timeout
                )));
            }
        };

        // Parse result
        let result = response.get("result").cloned().unwrap_or(Value::Null);
        serde_json::from_value(result).map_err(|e| LspError::InvalidResponse(e.to_string()))
    }

    /// Send a JSON-RPC notification (no response expected)
    async fn send_notification<P: Serialize>(
        &self,
        method: &str,
        params: P,
    ) -> Result<(), LspError> {
        let notification = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        });

        self.send_message(&notification).await
    }

    /// Send a JSON-RPC message
    async fn send_message(&self, message: &Value) -> Result<(), LspError> {
        let content = serde_json::to_string(message)?;
        let header = format!("Content-Length: {}\r\n\r\n", content.len());

        let mut stdin = self.stdin.lock().await;
        stdin.write_all(header.as_bytes()).await?;
        stdin.write_all(content.as_bytes()).await?;
        stdin.flush().await?;

        Ok(())
    }
}

impl Drop for LspClient {
    fn drop(&mut self) {
        // Abort the reader task to ensure clean shutdown
        if let Some(ref abort) = self.reader_abort {
            abort.abort();
        }
        // The process has kill_on_drop(true), so it will be cleaned up
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_lsp_client_creation_fails_for_nonexistent_server() {
        let result = LspClient::spawn("nonexistent-lsp-server-xyz", &[]).await;
        assert!(result.is_err());
    }

    #[test]
    fn test_url_from_file_path() {
        let path = Path::new("/tmp/test.ts");
        let url = Url::from_file_path(path);
        assert!(url.is_ok());
        assert!(url.unwrap().as_str().starts_with("file://"));
    }

    #[tokio::test]
    async fn test_spawn_with_custom_timeout() {
        let timeout = Duration::from_secs(60);
        let result = LspClient::spawn_with_timeout("nonexistent-server", &[], timeout).await;
        assert!(result.is_err());
    }

    #[test]
    fn test_default_timeout() {
        assert_eq!(DEFAULT_TIMEOUT_SECS, 30);
    }
}
