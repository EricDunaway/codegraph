//! LSP server lifecycle management (L5)
//!
//! This module handles the lifecycle of LSP server processes:
//! - Lazy spawn: servers are started only when needed
//! - Keep-alive: servers stay running during batch operations
//! - Graceful shutdown: clean termination after indexing

use crate::client::LspClient;
use crate::error::LspError;
use codegraph_types::LspServerConfig;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;
use url::Url;

/// Manages LSP server lifecycle (L5: lazy spawn, keep-alive, shutdown)
pub struct LspServerManager {
    config: LspServerConfig,
    client: Arc<Mutex<Option<LspClient>>>,
    root_uri: Arc<Mutex<Option<Url>>>,
}

impl LspServerManager {
    /// Create a new server manager with the given configuration
    pub fn new(config: LspServerConfig) -> Self {
        Self {
            config,
            client: Arc::new(Mutex::new(None)),
            root_uri: Arc::new(Mutex::new(None)),
        }
    }

    /// Check if server is currently running
    pub async fn is_running(&self) -> bool {
        self.client.lock().await.is_some()
    }

    /// Check if server is enabled in configuration
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    /// Get the server command
    pub fn server_command(&self) -> &str {
        &self.config.server
    }

    /// Ensure server is started (lazy spawn)
    ///
    /// This method starts the server if it's not already running.
    /// The server will be initialized with the given project root.
    pub async fn ensure_started(&self, project_root: &Path) -> Result<(), LspError> {
        if !self.config.enabled {
            return Err(LspError::UnsupportedLanguage(
                "LSP server not enabled".to_string(),
            ));
        }

        let mut client_guard = self.client.lock().await;
        if client_guard.is_none() {
            let root_url = Url::from_file_path(project_root)
                .map_err(|_| LspError::FileNotFound(project_root.display().to_string()))?;

            let client = LspClient::spawn(&self.config.server, &self.config.args).await?;

            // Initialize the server
            client.initialize(&root_url).await?;

            *client_guard = Some(client);
            *self.root_uri.lock().await = Some(root_url);
        }

        Ok(())
    }

    /// Ensure server is started with retry on failure
    ///
    /// Retries spawning the server up to `max_retries` times with exponential backoff.
    pub async fn ensure_started_with_retry(
        &self,
        project_root: &Path,
        max_retries: u32,
    ) -> Result<(), LspError> {
        let server_name = self.config.server.clone();
        let mut last_error = None;

        for attempt in 0..=max_retries {
            match self.ensure_started(project_root).await {
                Ok(()) => return Ok(()),
                Err(e) => {
                    log::warn!("LSP spawn attempt {} failed: {}", attempt + 1, e);
                    last_error = Some(e);

                    if attempt < max_retries {
                        // Exponential backoff: 100ms, 200ms, 400ms, ...
                        let delay = tokio::time::Duration::from_millis(100 << attempt);
                        tokio::time::sleep(delay).await;
                    }
                }
            }
        }

        Err(LspError::StartFailed(format!(
            "{} after {} retries: {}",
            server_name,
            max_retries,
            last_error.map(|e| e.to_string()).unwrap_or_default()
        )))
    }

    /// Get access to the client for making requests
    ///
    /// Returns the client wrapped in an Arc<Mutex<>> for concurrent access
    pub fn client(&self) -> Arc<Mutex<Option<LspClient>>> {
        Arc::clone(&self.client)
    }

    /// Shutdown the server gracefully
    pub async fn shutdown(&self) -> Result<(), LspError> {
        let mut client_guard = self.client.lock().await;
        if let Some(client) = client_guard.take() {
            client.shutdown().await?;
        }
        *self.root_uri.lock().await = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_config(server: &str) -> LspServerConfig {
        LspServerConfig {
            enabled: true,
            server: server.to_string(),
            args: vec!["--stdio".to_string()],
        }
    }

    #[tokio::test]
    async fn test_manager_creation() {
        let config = make_test_config("typescript-language-server");
        let manager = LspServerManager::new(config);

        assert!(!manager.is_running().await);
        assert!(manager.is_enabled());
    }

    #[tokio::test]
    async fn test_disabled_server_returns_error() {
        let config = LspServerConfig {
            enabled: false,
            server: "typescript-language-server".to_string(),
            args: vec![],
        };

        let manager = LspServerManager::new(config);
        let result = manager.ensure_started(Path::new("/tmp")).await;

        match result {
            Err(LspError::UnsupportedLanguage(_)) => {} // Expected
            Err(e) => panic!("Expected UnsupportedLanguage error, got: {}", e),
            Ok(_) => panic!("Expected error, got Ok"),
        }
    }

    #[tokio::test]
    async fn test_spawn_retry_with_backoff() {
        let config = make_test_config("nonexistent-server-that-will-fail");
        let manager = LspServerManager::new(config);

        let result = manager.ensure_started_with_retry(Path::new("/tmp"), 2).await;

        // Should fail after retries
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, LspError::StartFailed(_)));
    }

    #[tokio::test]
    async fn test_lazy_spawn_not_started_until_needed() {
        let config = make_test_config("typescript-language-server");
        let manager = LspServerManager::new(config);

        // Not spawned yet
        assert!(!manager.is_running().await);
    }

    #[tokio::test]
    async fn test_shutdown_when_not_running() {
        let config = make_test_config("typescript-language-server");
        let manager = LspServerManager::new(config);

        // Should not error when shutting down a non-running server
        let result = manager.shutdown().await;
        assert!(result.is_ok());
    }
}
