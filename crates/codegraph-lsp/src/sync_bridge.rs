//! Sync/Async bridge for calling LSP enrichers from sync code
//!
//! This module provides utilities for calling async LSP methods from
//! synchronous code paths (like the indexer and graph traversal).

use std::future::Future;
use tokio::runtime::{Builder, Runtime};

/// Bridge for calling async code from sync contexts
///
/// The indexer and graph traversal code is synchronous, but LSP enrichers
/// are async. This bridge provides a way to call async code from sync code.
pub struct LspSyncBridge {
    runtime: Runtime,
}

impl LspSyncBridge {
    /// Create a new sync bridge with a dedicated tokio runtime
    pub fn new() -> Result<Self, std::io::Error> {
        let runtime = Builder::new_current_thread().enable_all().build()?;
        Ok(Self { runtime })
    }

    /// Create a sync bridge with a multi-threaded runtime
    ///
    /// Use this when you need parallelism in async tasks.
    pub fn new_multi_thread(threads: usize) -> Result<Self, std::io::Error> {
        let runtime = Builder::new_multi_thread()
            .worker_threads(threads)
            .enable_all()
            .build()?;
        Ok(Self { runtime })
    }

    /// Block on an async future from sync code
    ///
    /// This runs the future to completion and returns its result.
    /// The current thread will block until the future completes.
    pub fn block_on<F: Future>(&self, future: F) -> F::Output {
        self.runtime.block_on(future)
    }

    /// Spawn a task on the runtime without blocking
    ///
    /// Returns a JoinHandle that can be used to await the result later.
    pub fn spawn<F>(&self, future: F) -> tokio::task::JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.runtime.spawn(future)
    }

    /// Get a handle to the runtime for manual task management
    pub fn handle(&self) -> tokio::runtime::Handle {
        self.runtime.handle().clone()
    }
}

impl Default for LspSyncBridge {
    fn default() -> Self {
        Self::new().expect("Failed to create tokio runtime")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sync_bridge_creation() {
        let bridge = LspSyncBridge::new();
        assert!(bridge.is_ok());
    }

    #[test]
    fn test_sync_bridge_block_on_simple() {
        let bridge = LspSyncBridge::new().unwrap();

        let result = bridge.block_on(async { 42 });

        assert_eq!(result, 42);
    }

    #[test]
    fn test_sync_bridge_block_on_with_sleep() {
        let bridge = LspSyncBridge::new().unwrap();

        let result = bridge.block_on(async {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            "done"
        });

        assert_eq!(result, "done");
    }

    #[test]
    fn test_sync_bridge_block_on_result() {
        let bridge = LspSyncBridge::new().unwrap();

        let result: Result<i32, &str> = bridge.block_on(async { Ok(42) });

        assert_eq!(result.unwrap(), 42);
    }

    #[test]
    fn test_sync_bridge_multiple_calls() {
        let bridge = LspSyncBridge::new().unwrap();

        let r1 = bridge.block_on(async { 1 });
        let r2 = bridge.block_on(async { 2 });
        let r3 = bridge.block_on(async { 3 });

        assert_eq!(r1 + r2 + r3, 6);
    }

    #[test]
    fn test_sync_bridge_default() {
        let bridge = LspSyncBridge::default();
        let result = bridge.block_on(async { "default works" });
        assert_eq!(result, "default works");
    }

    #[test]
    fn test_sync_bridge_with_io() {
        let bridge = LspSyncBridge::new().unwrap();

        // Simulate async I/O operation
        let result = bridge.block_on(async {
            let (tx, rx) = tokio::sync::oneshot::channel();
            tx.send(123).unwrap();
            rx.await.unwrap()
        });

        assert_eq!(result, 123);
    }

    #[test]
    fn test_multi_thread_bridge() {
        let bridge = LspSyncBridge::new_multi_thread(2).unwrap();

        let result = bridge.block_on(async {
            let handle = tokio::spawn(async { 42 });
            handle.await.unwrap()
        });

        assert_eq!(result, 42);
    }
}
