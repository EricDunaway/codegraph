//! Incremental sync for CodeGraph
//!
//! This crate handles detecting file changes and updating the graph incrementally.
//! It also provides git hooks management for automatic sync on commit.

pub mod change_detector;
pub mod edge_diff;
pub mod error;
pub mod git_hooks;
pub mod lock;
pub mod reembed;
pub mod selective;
pub mod sync;

pub use change_detector::{ChangeDetector, FileChange, ChangeKind};
pub use edge_diff::{EdgeDiff, EdgeKey, EdgeSnapshot};
pub use error::SyncError;
pub use git_hooks::GitHooksManager;
pub use lock::IndexLock;
pub use reembed::{check_reembed_triggers, should_full_reembed, ReembedConfig, ReembedReason};
pub use selective::SelectiveScope;
pub use sync::{SyncManager, SyncResult, SyncStats};
