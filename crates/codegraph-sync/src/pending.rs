//! Pending sync coalescing.
//!
//! When a git hook fires during an active sync, instead of losing the event,
//! write `sync.pending`. The running sync drains pending on completion.

use crate::error::SyncError;
use std::fs;
use std::path::{Path, PathBuf};

const ALLOWED_HOOKS: &[&str] = &[
    "post-commit",
    "post-checkout",
    "post-merge",
    "post-rewrite",
];

/// Manages `sync.pending` / `sync.processing` lifecycle.
///
/// Hook events that arrive while a sync is already running are written to
/// `sync.pending`. When the running sync finishes, it claims the pending
/// event (atomically renaming it to `sync.processing`) and re-syncs.
pub struct PendingSync {
    codegraph_dir: PathBuf,
}

impl PendingSync {
    pub fn new(codegraph_dir: &Path) -> Self {
        Self {
            codegraph_dir: codegraph_dir.to_path_buf(),
        }
    }

    fn pending_path(&self) -> PathBuf {
        self.codegraph_dir.join("sync.pending")
    }

    fn processing_path(&self) -> PathBuf {
        self.codegraph_dir.join("sync.processing")
    }

    /// Write a pending sync request for the given hook.
    ///
    /// Uses atomic write-then-rename to avoid partial reads.
    pub fn write(&self, hook_name: &str) -> Result<(), SyncError> {
        if !ALLOWED_HOOKS.contains(&hook_name) {
            return Err(SyncError::Other(format!(
                "Invalid hook name: {}",
                hook_name
            )));
        }
        let payload = format!(
            r#"{{"hook":"{}","timestamp":"{}"}}"#,
            hook_name,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
        );
        let temp_path = self.codegraph_dir.join("sync.pending.tmp");
        fs::write(&temp_path, &payload)?;
        fs::rename(&temp_path, self.pending_path())?;
        Ok(())
    }

    /// Atomically claim a pending event for processing.
    ///
    /// Returns `Some(hook_name)` if there was a pending event, `None` otherwise.
    pub fn claim(&self) -> Result<Option<String>, SyncError> {
        let pending = self.pending_path();
        if !pending.exists() {
            return Ok(None);
        }
        let processing = self.processing_path();
        fs::rename(&pending, &processing)?;
        let content = fs::read_to_string(&processing)?;
        let hook = content
            .split('"')
            .nth(3)
            .unwrap_or("unknown")
            .to_string();
        Ok(Some(hook))
    }

    /// Remove the processing marker after a sync completes.
    pub fn complete_processing(&self) -> Result<(), SyncError> {
        let processing = self.processing_path();
        if processing.exists() {
            fs::remove_file(&processing)?;
        }
        Ok(())
    }

    /// Check if there is a pending sync event.
    pub fn has_pending(&self) -> bool {
        self.pending_path().exists()
    }

    /// Remove both pending and processing markers (cleanup on error).
    pub fn cleanup(&self) -> Result<(), SyncError> {
        for path in [self.pending_path(), self.processing_path()] {
            if path.exists() {
                fs::remove_file(&path)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup() -> (TempDir, PendingSync) {
        let temp = TempDir::new().unwrap();
        let codegraph_dir = temp.path().join(".codegraph");
        fs::create_dir_all(&codegraph_dir).unwrap();
        let ps = PendingSync::new(&codegraph_dir);
        (temp, ps)
    }

    #[test]
    fn test_write_claim_complete_lifecycle() {
        let (_temp, ps) = setup();

        // Initially no pending
        assert!(!ps.has_pending());
        assert!(ps.claim().unwrap().is_none());

        // Write a pending event
        ps.write("post-commit").unwrap();
        assert!(ps.has_pending());

        // Claim it
        let hook = ps.claim().unwrap();
        assert_eq!(hook, Some("post-commit".to_string()));
        assert!(!ps.has_pending()); // pending is gone after claim
        assert!(ps.processing_path().exists()); // processing marker exists

        // Complete processing
        ps.complete_processing().unwrap();
        assert!(!ps.processing_path().exists());
    }

    #[test]
    fn test_has_pending() {
        let (_temp, ps) = setup();

        assert!(!ps.has_pending());
        ps.write("post-merge").unwrap();
        assert!(ps.has_pending());

        // Claim removes pending
        ps.claim().unwrap();
        assert!(!ps.has_pending());
    }

    #[test]
    fn test_invalid_hook_name_error() {
        let (_temp, ps) = setup();

        let result = ps.write("pre-commit");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("Invalid hook name"));
    }

    #[test]
    fn test_cleanup_removes_both_files() {
        let (_temp, ps) = setup();

        ps.write("post-commit").unwrap();
        ps.claim().unwrap();
        // Now we have processing but no pending; write another pending
        ps.write("post-merge").unwrap();
        assert!(ps.pending_path().exists());
        assert!(ps.processing_path().exists());

        ps.cleanup().unwrap();
        assert!(!ps.pending_path().exists());
        assert!(!ps.processing_path().exists());
    }

    #[test]
    fn test_write_overwrites_previous_pending() {
        let (_temp, ps) = setup();

        ps.write("post-commit").unwrap();
        ps.write("post-checkout").unwrap();

        let hook = ps.claim().unwrap();
        assert_eq!(hook, Some("post-checkout".to_string()));
    }
}
