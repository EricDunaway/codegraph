//! Git hooks management for automatic sync

use crate::error::SyncError;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// Git hook names
const POST_COMMIT_HOOK: &str = "post-commit";
const POST_CHECKOUT_HOOK: &str = "post-checkout";
const POST_MERGE_HOOK: &str = "post-merge";

/// Hook script template
const HOOK_SCRIPT: &str = r#"#!/bin/sh
# CodeGraph auto-sync hook
# This hook was installed by codegraph hooks install

# Run codegraph sync in the background
codegraph sync "$PWD" &
"#;

/// Manages git hooks for automatic sync
pub struct GitHooksManager {
    /// Path to .git/hooks directory
    hooks_dir: PathBuf,
}

impl GitHooksManager {
    /// Create a new hooks manager
    pub fn new(repo_path: impl AsRef<Path>) -> Result<Self, SyncError> {
        let repo_path = repo_path.as_ref().to_path_buf();
        let git_dir = repo_path.join(".git");

        if !git_dir.exists() {
            return Err(SyncError::Git {
                message: "Not a git repository (no .git directory)".to_string(),
            });
        }

        let hooks_dir = git_dir.join("hooks");

        Ok(Self {
            hooks_dir,
        })
    }

    /// Install all CodeGraph hooks
    pub fn install_all(&self) -> Result<(), SyncError> {
        self.ensure_hooks_dir()?;

        self.install_hook(POST_COMMIT_HOOK)?;
        self.install_hook(POST_CHECKOUT_HOOK)?;
        self.install_hook(POST_MERGE_HOOK)?;

        log::info!("Installed all CodeGraph git hooks");
        Ok(())
    }

    /// Install a specific hook
    pub fn install_hook(&self, hook_name: &str) -> Result<(), SyncError> {
        let hook_path = self.hooks_dir.join(hook_name);

        if hook_path.exists() {
            // Check if it's our hook or a user hook
            let content = fs::read_to_string(&hook_path)?;
            if content.contains("CodeGraph auto-sync hook") {
                // Already installed
                return Ok(());
            }

            // Backup existing hook
            let backup_path = self.hooks_dir.join(format!("{}.codegraph-backup", hook_name));
            fs::rename(&hook_path, &backup_path)?;
            log::info!("Backed up existing {} hook", hook_name);
        }

        // Write our hook
        fs::write(&hook_path, HOOK_SCRIPT)?;

        // Make executable
        #[cfg(unix)]
        {
            let mut perms = fs::metadata(&hook_path)?.permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&hook_path, perms)?;
        }

        log::info!("Installed {} hook", hook_name);
        Ok(())
    }

    /// Uninstall all CodeGraph hooks
    pub fn uninstall_all(&self) -> Result<(), SyncError> {
        self.uninstall_hook(POST_COMMIT_HOOK)?;
        self.uninstall_hook(POST_CHECKOUT_HOOK)?;
        self.uninstall_hook(POST_MERGE_HOOK)?;

        log::info!("Uninstalled all CodeGraph git hooks");
        Ok(())
    }

    /// Uninstall a specific hook
    pub fn uninstall_hook(&self, hook_name: &str) -> Result<(), SyncError> {
        let hook_path = self.hooks_dir.join(hook_name);

        if !hook_path.exists() {
            return Ok(());
        }

        // Check if it's our hook
        let content = fs::read_to_string(&hook_path)?;
        if !content.contains("CodeGraph auto-sync hook") {
            // Not our hook, don't remove
            return Ok(());
        }

        // Remove our hook
        fs::remove_file(&hook_path)?;

        // Restore backup if exists
        let backup_path = self.hooks_dir.join(format!("{}.codegraph-backup", hook_name));
        if backup_path.exists() {
            fs::rename(&backup_path, &hook_path)?;
            log::info!("Restored backed up {} hook", hook_name);
        }

        log::info!("Uninstalled {} hook", hook_name);
        Ok(())
    }

    /// Check if hooks are installed
    pub fn is_installed(&self) -> bool {
        self.is_hook_installed(POST_COMMIT_HOOK)
    }

    /// Check if a specific hook is installed
    pub fn is_hook_installed(&self, hook_name: &str) -> bool {
        let hook_path = self.hooks_dir.join(hook_name);

        if !hook_path.exists() {
            return false;
        }

        match fs::read_to_string(&hook_path) {
            Ok(content) => content.contains("CodeGraph auto-sync hook"),
            Err(_) => false,
        }
    }

    /// List all installed CodeGraph hooks
    pub fn list_installed(&self) -> Vec<String> {
        let mut installed = Vec::new();

        for hook in [POST_COMMIT_HOOK, POST_CHECKOUT_HOOK, POST_MERGE_HOOK] {
            if self.is_hook_installed(hook) {
                installed.push(hook.to_string());
            }
        }

        installed
    }

    /// Ensure hooks directory exists
    fn ensure_hooks_dir(&self) -> Result<(), SyncError> {
        if !self.hooks_dir.exists() {
            fs::create_dir_all(&self.hooks_dir)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_git_repo() -> TempDir {
        let dir = tempfile::tempdir().unwrap();

        // Create .git directory
        let git_dir = dir.path().join(".git");
        fs::create_dir(&git_dir).unwrap();

        // Create hooks directory
        let hooks_dir = git_dir.join("hooks");
        fs::create_dir(&hooks_dir).unwrap();

        dir
    }

    #[test]
    fn test_not_a_git_repo() {
        let dir = tempfile::tempdir().unwrap();
        let result = GitHooksManager::new(dir.path());
        assert!(result.is_err());
    }

    #[test]
    fn test_install_and_check() {
        let dir = setup_git_repo();
        let manager = GitHooksManager::new(dir.path()).unwrap();

        assert!(!manager.is_installed());

        manager.install_all().unwrap();

        assert!(manager.is_installed());
        assert!(manager.is_hook_installed(POST_COMMIT_HOOK));
        assert!(manager.is_hook_installed(POST_CHECKOUT_HOOK));
        assert!(manager.is_hook_installed(POST_MERGE_HOOK));
    }

    #[test]
    fn test_list_installed() {
        let dir = setup_git_repo();
        let manager = GitHooksManager::new(dir.path()).unwrap();

        assert!(manager.list_installed().is_empty());

        manager.install_hook(POST_COMMIT_HOOK).unwrap();

        let installed = manager.list_installed();
        assert_eq!(installed.len(), 1);
        assert!(installed.contains(&POST_COMMIT_HOOK.to_string()));
    }

    #[test]
    fn test_uninstall() {
        let dir = setup_git_repo();
        let manager = GitHooksManager::new(dir.path()).unwrap();

        manager.install_all().unwrap();
        assert!(manager.is_installed());

        manager.uninstall_all().unwrap();
        assert!(!manager.is_installed());
    }

    #[test]
    fn test_backup_existing_hook() {
        let dir = setup_git_repo();
        let hooks_dir = dir.path().join(".git/hooks");

        // Create a pre-existing hook
        let hook_path = hooks_dir.join(POST_COMMIT_HOOK);
        fs::write(&hook_path, "#!/bin/sh\necho 'user hook'\n").unwrap();

        let manager = GitHooksManager::new(dir.path()).unwrap();
        manager.install_hook(POST_COMMIT_HOOK).unwrap();

        // Backup should exist
        let backup_path = hooks_dir.join(format!("{}.codegraph-backup", POST_COMMIT_HOOK));
        assert!(backup_path.exists());

        // Our hook should be installed
        assert!(manager.is_hook_installed(POST_COMMIT_HOOK));
    }

    #[test]
    fn test_restore_backup_on_uninstall() {
        let dir = setup_git_repo();
        let hooks_dir = dir.path().join(".git/hooks");

        // Create a pre-existing hook
        let hook_path = hooks_dir.join(POST_COMMIT_HOOK);
        let original_content = "#!/bin/sh\necho 'user hook'\n";
        fs::write(&hook_path, original_content).unwrap();

        let manager = GitHooksManager::new(dir.path()).unwrap();
        manager.install_hook(POST_COMMIT_HOOK).unwrap();
        manager.uninstall_hook(POST_COMMIT_HOOK).unwrap();

        // Original hook should be restored
        let restored_content = fs::read_to_string(&hook_path).unwrap();
        assert_eq!(restored_content, original_content);
    }
}
