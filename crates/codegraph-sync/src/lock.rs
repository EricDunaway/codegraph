//! File-based locking for CodeGraph indexing (I12)
//!
//! Prevents concurrent indexing operations on the same project.
//! Uses a lock file with mtime-based stale detection.

use crate::error::SyncError;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Default stale lock timeout (5 minutes)
const STALE_LOCK_TIMEOUT: Duration = Duration::from_secs(300);

/// Lock for exclusive indexing access to a CodeGraph project
#[derive(Debug)]
pub struct IndexLock {
    path: PathBuf,
    held: bool,
}

impl IndexLock {
    /// Acquire the lock, blocking until available
    ///
    /// If a stale lock is detected (mtime older than STALE_LOCK_TIMEOUT),
    /// it will be automatically broken.
    pub fn acquire(codegraph_dir: &Path) -> Result<Self, SyncError> {
        let lock_path = codegraph_dir.join("index.lock");

        // Check for stale lock
        if lock_path.exists() {
            if is_stale_lock(&lock_path)? {
                log::warn!("Breaking stale lock file: {}", lock_path.display());
                fs::remove_file(&lock_path)?;
            } else {
                return Err(SyncError::LockHeld);
            }
        }

        // Create lock file with exclusive access
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::AlreadyExists {
                    SyncError::LockHeld
                } else {
                    SyncError::LockFailed(e.to_string())
                }
            })?;

        // Write PID to lock file for debugging
        let pid = std::process::id();
        writeln!(file, "{}", pid)?;

        Ok(Self {
            path: lock_path,
            held: true,
        })
    }

    /// Try to acquire the lock without blocking
    ///
    /// Returns `Err(SyncError::LockHeld)` if lock is already held.
    pub fn try_acquire(codegraph_dir: &Path) -> Result<Self, SyncError> {
        Self::acquire(codegraph_dir)
    }

    /// Attempt to acquire lock. On LockHeld in hook mode, write sync.pending.
    /// Returns Ok(Some(lock)) on success, Ok(None) if pending was written.
    pub fn try_acquire_or_pending(
        codegraph_dir: &Path,
        hook_name: &str,
    ) -> Result<Option<Self>, SyncError> {
        match Self::acquire(codegraph_dir) {
            Ok(lock) => Ok(Some(lock)),
            Err(SyncError::LockHeld) => {
                use crate::pending::PendingSync;
                PendingSync::new(codegraph_dir).write(hook_name)?;
                Ok(None)
            }
            Err(e) => Err(e),
        }
    }

    /// Check if this lock is currently held
    pub fn is_held(&self) -> bool {
        self.held
    }

    /// Refresh the lock's mtime to prevent it from being considered stale
    pub fn refresh(&self) -> Result<(), SyncError> {
        if !self.held {
            return Err(SyncError::LockFailed("Lock not held".to_string()));
        }

        // Touch the file to update mtime
        let now = filetime::FileTime::now();
        filetime::set_file_mtime(&self.path, now)?;
        Ok(())
    }

    /// Release the lock explicitly
    pub fn release(mut self) -> Result<(), SyncError> {
        self.release_internal()
    }

    fn release_internal(&mut self) -> Result<(), SyncError> {
        if self.held {
            self.held = false;
            if self.path.exists() {
                fs::remove_file(&self.path)?;
            }
        }
        Ok(())
    }
}

impl Drop for IndexLock {
    fn drop(&mut self) {
        if self.held {
            if let Err(e) = self.release_internal() {
                log::error!("Failed to release lock on drop: {}", e);
            }
        }
    }
}

/// Check if a lock file is stale based on its mtime
fn is_stale_lock(lock_path: &Path) -> Result<bool, SyncError> {
    let metadata = fs::metadata(lock_path)?;
    let modified = metadata.modified()?;
    let age = SystemTime::now()
        .duration_since(modified)
        .unwrap_or(Duration::ZERO);

    Ok(age > STALE_LOCK_TIMEOUT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_acquire_index_lock() {
        let temp = TempDir::new().unwrap();
        let codegraph_dir = temp.path().join(".codegraph");
        fs::create_dir_all(&codegraph_dir).unwrap();

        let lock = IndexLock::acquire(&codegraph_dir).unwrap();
        assert!(lock.is_held());

        // Lock file should exist
        assert!(codegraph_dir.join("index.lock").exists());
    }

    #[test]
    fn test_lock_prevents_concurrent_access() {
        let temp = TempDir::new().unwrap();
        let codegraph_dir = temp.path().join(".codegraph");
        fs::create_dir_all(&codegraph_dir).unwrap();

        let lock1 = IndexLock::acquire(&codegraph_dir).unwrap();

        // Second acquire should fail
        let lock2 = IndexLock::try_acquire(&codegraph_dir);
        assert!(lock2.is_err());
        match lock2 {
            Err(SyncError::LockHeld) => {}
            other => panic!("Expected LockHeld error, got {:?}", other),
        }

        drop(lock1);

        // Now it should succeed
        let lock3 = IndexLock::acquire(&codegraph_dir).unwrap();
        assert!(lock3.is_held());
    }

    #[test]
    fn test_stale_lock_detected_by_mtime() {
        let temp = TempDir::new().unwrap();
        let codegraph_dir = temp.path().join(".codegraph");
        fs::create_dir_all(&codegraph_dir).unwrap();

        // Create stale lock file (old mtime)
        let lock_path = codegraph_dir.join("index.lock");
        fs::write(&lock_path, "stale").unwrap();

        // Set mtime to 10 minutes ago (beyond 5 min timeout)
        let old_time = SystemTime::now() - Duration::from_secs(600);
        filetime::set_file_mtime(&lock_path, filetime::FileTime::from_system_time(old_time))
            .unwrap();

        // Should be able to acquire (stale lock broken)
        let lock = IndexLock::acquire(&codegraph_dir).unwrap();
        assert!(lock.is_held());
    }

    #[test]
    fn test_lock_released_on_drop() {
        let temp = TempDir::new().unwrap();
        let codegraph_dir = temp.path().join(".codegraph");
        fs::create_dir_all(&codegraph_dir).unwrap();

        let lock_path = codegraph_dir.join("index.lock");

        {
            let lock = IndexLock::acquire(&codegraph_dir).unwrap();
            assert!(lock.is_held());
            assert!(lock_path.exists());
        }

        // Lock file should be gone after drop
        assert!(!lock_path.exists());
    }

    #[test]
    fn test_lock_refresh() {
        let temp = TempDir::new().unwrap();
        let codegraph_dir = temp.path().join(".codegraph");
        fs::create_dir_all(&codegraph_dir).unwrap();

        let lock = IndexLock::acquire(&codegraph_dir).unwrap();
        let lock_path = codegraph_dir.join("index.lock");

        // Get initial mtime
        let initial_mtime = fs::metadata(&lock_path).unwrap().modified().unwrap();

        // Wait a tiny bit and refresh
        std::thread::sleep(Duration::from_millis(10));
        lock.refresh().unwrap();

        // mtime should be updated
        let new_mtime = fs::metadata(&lock_path).unwrap().modified().unwrap();
        assert!(new_mtime > initial_mtime);
    }

    #[test]
    fn test_lock_contains_pid() {
        let temp = TempDir::new().unwrap();
        let codegraph_dir = temp.path().join(".codegraph");
        fs::create_dir_all(&codegraph_dir).unwrap();

        let _lock = IndexLock::acquire(&codegraph_dir).unwrap();
        let lock_path = codegraph_dir.join("index.lock");

        let contents = fs::read_to_string(&lock_path).unwrap();
        let pid: u32 = contents.trim().parse().unwrap();
        assert_eq!(pid, std::process::id());
    }

    #[test]
    fn test_try_acquire_or_pending_succeeds_when_unlocked() {
        let temp = TempDir::new().unwrap();
        let codegraph_dir = temp.path().join(".codegraph");
        fs::create_dir_all(&codegraph_dir).unwrap();

        let result = IndexLock::try_acquire_or_pending(&codegraph_dir, "post-commit").unwrap();
        assert!(result.is_some());
        assert!(result.unwrap().is_held());
    }

    #[test]
    fn test_try_acquire_or_pending_writes_pending_on_collision() {
        let temp = TempDir::new().unwrap();
        let codegraph_dir = temp.path().join(".codegraph");
        fs::create_dir_all(&codegraph_dir).unwrap();

        // Hold the lock
        let _lock = IndexLock::acquire(&codegraph_dir).unwrap();

        // try_acquire_or_pending should write sync.pending and return None
        let result =
            IndexLock::try_acquire_or_pending(&codegraph_dir, "post-commit").unwrap();
        assert!(result.is_none());

        // sync.pending should exist
        assert!(codegraph_dir.join("sync.pending").exists());
    }
}
