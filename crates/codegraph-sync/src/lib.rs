//! Incremental sync for CodeGraph
//!
//! This crate handles detecting file changes and updating the graph incrementally.
//! It also provides git hooks management for automatic sync on commit.

pub mod change_detector;
pub mod error;
pub mod git_hooks;
pub mod sync;

pub use change_detector::{ChangeDetector, FileChange, ChangeKind};
pub use error::SyncError;
pub use git_hooks::GitHooksManager;
pub use sync::{SyncManager, SyncResult, SyncStats};
