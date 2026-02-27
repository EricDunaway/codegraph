//! Git hooks management for automatic sync

use crate::error::SyncError;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

/// All hooks managed by CodeGraph
const HOOKS: &[&str] = &[
    "post-commit",
    "post-checkout",
    "post-merge",
    "post-rewrite",
];

/// Marker line used to identify CodeGraph-managed hooks
const HOOK_MARKER: &str = "Managed by codegraph";

/// Backup suffix for existing hooks
const BACKUP_SUFFIX: &str = ".codegraph-orig";

/// Generate the hook script for a given hook name
fn hook_script(_hook_name: &str) -> String {
    r#"#!/bin/sh
# Managed by codegraph — do not edit this block

git rev-parse --is-inside-work-tree >/dev/null 2>&1 || exit 0
repo_root="$(git rev-parse --show-toplevel 2>/dev/null)" || exit 0
[ -d "$repo_root/.codegraph" ] || exit 0

hook_name="$(basename "$0")"

# Chain to original hook if it exists
orig_hook="$(dirname "$0")/${hook_name}.codegraph-orig"
orig_exit=0
if [ -x "$orig_hook" ]; then
    "$orig_hook" "$@" || orig_exit=$?
fi

# Trigger sync in background
if command -v codegraph >/dev/null 2>&1; then
    ( codegraph sync "$repo_root" --hook "$hook_name" \
        >> "$repo_root/.codegraph/sync.log" 2>&1 ) &
fi

exit "$orig_exit"
"#.to_string()
}

/// Resolve the hooks directory using `git rev-parse --git-path hooks`.
///
/// Falls back to `<repo_root>/.git/hooks` if git is unavailable.
fn get_hooks_dir(repo_root: &Path) -> Result<PathBuf, SyncError> {
    let output = Command::new("git")
        .args(["rev-parse", "--git-path", "hooks"])
        .current_dir(repo_root)
        .output()
        .map_err(|e| SyncError::Other(format!("git rev-parse failed: {}", e)))?;
    if !output.status.success() {
        return Err(SyncError::Other("Not a git repository".to_string()));
    }
    let hooks_path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    // If path is relative, it's relative to repo root
    let hooks_dir = if Path::new(&hooks_path).is_absolute() {
        PathBuf::from(hooks_path)
    } else {
        repo_root.join(hooks_path)
    };
    Ok(hooks_dir)
}

/// Detect third-party hook managers (Husky, Lefthook).
fn detect_hook_manager(repo_root: &Path) -> Option<String> {
    // Check for Husky
    if repo_root.join(".husky").exists() {
        return Some("Husky".to_string());
    }
    // Check for Lefthook
    if repo_root.join("lefthook.yml").exists() || repo_root.join(".lefthook.yml").exists() {
        return Some("Lefthook".to_string());
    }
    None
}

/// Manages git hooks for automatic sync
pub struct GitHooksManager {
    /// Root of the git repository
    repo_root: PathBuf,
    /// Path to the hooks directory
    hooks_dir: PathBuf,
    /// Whether to force install even if a hook manager is detected
    force: bool,
}

impl GitHooksManager {
    /// Create a new hooks manager.
    ///
    /// Uses `git rev-parse --git-path hooks` to locate the hooks directory,
    /// which works correctly with worktrees and custom `core.hooksPath`.
    pub fn new(repo_path: impl AsRef<Path>, force: bool) -> Result<Self, SyncError> {
        let repo_root = repo_path.as_ref().to_path_buf();
        let hooks_dir = get_hooks_dir(&repo_root)?;

        Ok(Self {
            repo_root,
            hooks_dir,
            force,
        })
    }

    /// Create a hooks manager with an explicit hooks directory (for testing).
    #[cfg(test)]
    fn with_hooks_dir(repo_root: PathBuf, hooks_dir: PathBuf, force: bool) -> Self {
        Self {
            repo_root,
            hooks_dir,
            force,
        }
    }

    /// Install all CodeGraph hooks.
    ///
    /// Checks for third-party hook managers (Husky, Lefthook) and refuses
    /// unless `force` was set. Also refuses if a `.codegraph-orig` backup
    /// already exists (indicates a previous incomplete install/uninstall).
    pub fn install_all(&self) -> Result<(), SyncError> {
        // Check for hook managers
        if !self.force {
            if let Some(tool) = detect_hook_manager(&self.repo_root) {
                return Err(SyncError::HookManagerDetected { tool });
            }
        }

        self.ensure_hooks_dir()?;

        for hook in HOOKS {
            self.install_hook(hook)?;
        }

        log::info!("Installed all CodeGraph git hooks");
        Ok(())
    }

    /// Install a specific hook
    pub fn install_hook(&self, hook_name: &str) -> Result<(), SyncError> {
        if !HOOKS.contains(&hook_name) {
            return Err(SyncError::InvalidHookName { name: hook_name.to_string() });
        }
        let hook_path = self.hooks_dir.join(hook_name);
        let backup_path = self.hooks_dir.join(format!("{}{}", hook_name, BACKUP_SUFFIX));

        // Conflict detection: refuse if backup already exists
        if backup_path.exists() {
            return Err(SyncError::HookInstallFailed {
                hook: hook_name.to_string(),
                message: format!(
                    "Backup file {} already exists. \
                     This indicates a previous incomplete install/uninstall. \
                     Remove it manually and retry.",
                    backup_path.display()
                ),
            });
        }

        if hook_path.exists() {
            // Check if it's our hook or a user hook
            let content = fs::read_to_string(&hook_path)?;
            if content.contains(HOOK_MARKER) {
                // Already installed — update in place
                fs::write(&hook_path, hook_script(hook_name))?;
                return Ok(());
            }

            // Backup existing hook
            fs::rename(&hook_path, &backup_path)?;
            log::info!("Backed up existing {} hook to {}", hook_name, backup_path.display());
        }

        // Write our hook
        fs::write(&hook_path, hook_script(hook_name))?;

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
        for hook in HOOKS {
            self.uninstall_hook(hook)?;
        }

        log::info!("Uninstalled all CodeGraph git hooks");
        Ok(())
    }

    /// Uninstall a specific hook
    pub fn uninstall_hook(&self, hook_name: &str) -> Result<(), SyncError> {
        if !HOOKS.contains(&hook_name) {
            return Err(SyncError::InvalidHookName { name: hook_name.to_string() });
        }
        let hook_path = self.hooks_dir.join(hook_name);

        if !hook_path.exists() {
            return Ok(());
        }

        // Check if it's our hook
        let content = fs::read_to_string(&hook_path)?;
        if !content.contains(HOOK_MARKER) {
            // Not our hook, don't remove
            return Ok(());
        }

        // Remove our hook
        fs::remove_file(&hook_path)?;

        // Restore backup if exists
        let backup_path = self.hooks_dir.join(format!("{}{}", hook_name, BACKUP_SUFFIX));
        if backup_path.exists() {
            fs::rename(&backup_path, &hook_path)?;
            log::info!("Restored backed up {} hook", hook_name);
        }

        log::info!("Uninstalled {} hook", hook_name);
        Ok(())
    }

    /// Check if hooks are installed
    pub fn is_installed(&self) -> bool {
        self.is_hook_installed(HOOKS[0])
    }

    /// Check if a specific hook is installed
    pub fn is_hook_installed(&self, hook_name: &str) -> bool {
        if !HOOKS.contains(&hook_name) {
            return false;
        }
        let hook_path = self.hooks_dir.join(hook_name);

        if !hook_path.exists() {
            return false;
        }

        match fs::read_to_string(&hook_path) {
            Ok(content) => content.contains(HOOK_MARKER),
            Err(_) => false,
        }
    }

    /// List all installed CodeGraph hooks
    pub fn list_installed(&self) -> Vec<String> {
        let mut installed = Vec::new();

        for hook in HOOKS {
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

    /// Helper to create a GitHooksManager for tests (bypasses git rev-parse)
    fn test_manager(dir: &TempDir, force: bool) -> GitHooksManager {
        let repo_root = dir.path().to_path_buf();
        let hooks_dir = repo_root.join(".git/hooks");
        GitHooksManager::with_hooks_dir(repo_root, hooks_dir, force)
    }

    #[test]
    fn test_install_and_check() {
        let dir = setup_git_repo();
        let manager = test_manager(&dir, false);

        assert!(!manager.is_installed());

        manager.install_all().unwrap();

        assert!(manager.is_installed());
        assert!(manager.is_hook_installed("post-commit"));
        assert!(manager.is_hook_installed("post-checkout"));
        assert!(manager.is_hook_installed("post-merge"));
        assert!(manager.is_hook_installed("post-rewrite"));
    }

    #[test]
    fn test_list_installed() {
        let dir = setup_git_repo();
        let manager = test_manager(&dir, false);

        assert!(manager.list_installed().is_empty());

        manager.install_hook("post-commit").unwrap();

        let installed = manager.list_installed();
        assert_eq!(installed.len(), 1);
        assert!(installed.contains(&"post-commit".to_string()));
    }

    #[test]
    fn test_uninstall() {
        let dir = setup_git_repo();
        let manager = test_manager(&dir, false);

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
        let hook_path = hooks_dir.join("post-commit");
        fs::write(&hook_path, "#!/bin/sh\necho 'user hook'\n").unwrap();

        let manager = test_manager(&dir, false);
        manager.install_hook("post-commit").unwrap();

        // Backup should exist with new suffix
        let backup_path = hooks_dir.join(format!("post-commit{}", BACKUP_SUFFIX));
        assert!(backup_path.exists());

        // Our hook should be installed
        assert!(manager.is_hook_installed("post-commit"));
    }

    #[test]
    fn test_restore_backup_on_uninstall() {
        let dir = setup_git_repo();
        let hooks_dir = dir.path().join(".git/hooks");

        // Create a pre-existing hook
        let hook_path = hooks_dir.join("post-commit");
        let original_content = "#!/bin/sh\necho 'user hook'\n";
        fs::write(&hook_path, original_content).unwrap();

        let manager = test_manager(&dir, false);
        manager.install_hook("post-commit").unwrap();
        manager.uninstall_hook("post-commit").unwrap();

        // Original hook should be restored
        let restored_content = fs::read_to_string(&hook_path).unwrap();
        assert_eq!(restored_content, original_content);
    }

    #[test]
    fn test_hook_script_contains_hook_name() {
        let dir = setup_git_repo();
        let manager = test_manager(&dir, false);

        manager.install_hook("post-commit").unwrap();

        let hook_path = dir.path().join(".git/hooks/post-commit");
        let content = fs::read_to_string(&hook_path).unwrap();
        assert!(content.contains("--hook \"$hook_name\""));
        assert!(content.contains(HOOK_MARKER));
    }

    #[test]
    fn test_hook_script_per_hook_name() {
        let dir = setup_git_repo();
        let manager = test_manager(&dir, false);

        manager.install_all().unwrap();

        for hook in HOOKS {
            let hook_path = dir.path().join(format!(".git/hooks/{}", hook));
            let content = fs::read_to_string(&hook_path).unwrap();
            assert!(
                content.contains("--hook \"$hook_name\""),
                "Hook {} should contain --hook \"$hook_name\"",
                hook,
            );
            assert!(
                content.contains(HOOK_MARKER),
                "Hook {} should contain the marker",
                hook,
            );
        }
    }

    #[test]
    fn test_detect_husky() {
        let dir = setup_git_repo();
        fs::create_dir(dir.path().join(".husky")).unwrap();
        assert_eq!(detect_hook_manager(dir.path()), Some("Husky".to_string()));
    }

    #[test]
    fn test_detect_lefthook() {
        let dir = setup_git_repo();
        fs::write(dir.path().join("lefthook.yml"), "").unwrap();
        assert_eq!(
            detect_hook_manager(dir.path()),
            Some("Lefthook".to_string())
        );
    }

    #[test]
    fn test_detect_lefthook_dotfile() {
        let dir = setup_git_repo();
        fs::write(dir.path().join(".lefthook.yml"), "").unwrap();
        assert_eq!(
            detect_hook_manager(dir.path()),
            Some("Lefthook".to_string())
        );
    }

    #[test]
    fn test_no_hook_manager() {
        let dir = setup_git_repo();
        assert_eq!(detect_hook_manager(dir.path()), None);
    }

    #[test]
    fn test_hook_manager_blocks_install() {
        let dir = setup_git_repo();
        fs::create_dir(dir.path().join(".husky")).unwrap();

        let manager = test_manager(&dir, false);
        let result = manager.install_all();
        assert!(result.is_err());
        match result {
            Err(SyncError::HookManagerDetected { tool }) => {
                assert_eq!(tool, "Husky");
            }
            other => panic!("Expected HookManagerDetected, got {:?}", other),
        }
    }

    #[test]
    fn test_force_bypasses_hook_manager() {
        let dir = setup_git_repo();
        fs::create_dir(dir.path().join(".husky")).unwrap();

        let manager = test_manager(&dir, true);
        manager.install_all().unwrap();
        assert!(manager.is_installed());
    }

    #[test]
    fn test_conflict_detection_backup_exists() {
        let dir = setup_git_repo();
        let hooks_dir = dir.path().join(".git/hooks");

        // Create a backup file that shouldn't exist
        let backup_path = hooks_dir.join(format!("post-commit{}", BACKUP_SUFFIX));
        fs::write(&backup_path, "stale backup").unwrap();

        let manager = test_manager(&dir, false);
        let result = manager.install_hook("post-commit");
        assert!(result.is_err());
        match result {
            Err(SyncError::HookInstallFailed { hook, message }) => {
                assert_eq!(hook, "post-commit");
                assert!(message.contains("already exists"));
            }
            other => panic!("Expected HookInstallFailed, got {:?}", other),
        }
    }

    #[test]
    fn test_reinstall_updates_existing_hook() {
        let dir = setup_git_repo();
        let manager = test_manager(&dir, false);

        // Install once
        manager.install_hook("post-commit").unwrap();

        // Install again — should succeed (update in place)
        manager.install_hook("post-commit").unwrap();

        // Still installed
        assert!(manager.is_hook_installed("post-commit"));
    }

    #[test]
    fn test_post_rewrite_in_hooks_list() {
        assert!(HOOKS.contains(&"post-rewrite"));
    }
}
