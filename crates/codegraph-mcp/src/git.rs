//! Git utilities for staleness detection

use std::collections::HashSet;
use std::path::Path;
use std::process::Command;

/// Result of checking git status
#[derive(Debug, Default)]
pub struct GitStatus {
    /// Files that have been modified (staged or unstaged)
    pub modified: HashSet<String>,
    /// Files that are untracked
    pub untracked: HashSet<String>,
    /// Files that have been deleted
    pub deleted: HashSet<String>,
    /// Whether git is available and this is a git repo
    pub is_git_repo: bool,
}

impl GitStatus {
    /// Get all dirty files (modified, untracked, or deleted)
    pub fn dirty_files(&self) -> HashSet<&String> {
        self.modified
            .iter()
            .chain(self.untracked.iter())
            .chain(self.deleted.iter())
            .collect()
    }

    /// Check if a file path is dirty
    pub fn is_dirty(&self, file_path: &str) -> bool {
        // Normalize path for comparison
        let normalized = file_path.trim_start_matches("./");
        self.modified.contains(normalized)
            || self.untracked.contains(normalized)
            || self.deleted.contains(normalized)
    }

    /// Count of dirty files
    pub fn dirty_count(&self) -> usize {
        self.modified.len() + self.untracked.len() + self.deleted.len()
    }
}

/// Get git status for a repository
pub fn get_git_status(repo_path: &Path) -> GitStatus {
    let output = Command::new("git")
        .args(["status", "--porcelain", "-uall"])
        .current_dir(repo_path)
        .output();

    let output = match output {
        Ok(o) if o.status.success() => o,
        _ => return GitStatus::default(),
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut status = GitStatus {
        is_git_repo: true,
        ..Default::default()
    };

    for line in stdout.lines() {
        if line.len() < 4 {
            continue;
        }

        let status_code = &line[0..2];
        let file_path = line[3..].to_string();

        match status_code.trim() {
            "M" | "MM" | "AM" | " M" | "A" => {
                status.modified.insert(file_path);
            }
            "??" => {
                status.untracked.insert(file_path);
            }
            "D" | " D" | "AD" => {
                status.deleted.insert(file_path);
            }
            "R" | "RM" => {
                // Renamed - the new name is after " -> "
                if let Some(pos) = file_path.find(" -> ") {
                    status.modified.insert(file_path[pos + 4..].to_string());
                } else {
                    status.modified.insert(file_path);
                }
            }
            _ => {
                // Other statuses (e.g., "C" for copied) - treat as modified
                if !status_code.trim().is_empty() {
                    status.modified.insert(file_path);
                }
            }
        }
    }

    status
}

/// Check if git hooks are installed for CodeGraph
pub fn are_hooks_installed(repo_path: &Path) -> bool {
    let hooks_dir = repo_path.join(".git/hooks");

    for hook in ["post-commit", "post-checkout", "post-merge"] {
        let hook_path = hooks_dir.join(hook);
        if let Ok(content) = std::fs::read_to_string(&hook_path) {
            if content.contains("CodeGraph auto-sync hook") {
                return true;
            }
        }
    }

    false
}

/// Get the time of the last sync (based on database mtime)
pub fn get_last_sync_time(repo_path: &Path) -> Option<std::time::SystemTime> {
    let db_path = repo_path.join(".codegraph/codegraph.db");
    std::fs::metadata(&db_path)
        .ok()
        .and_then(|m| m.modified().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_git_status_dirty_check() {
        let mut status = GitStatus::default();
        status.modified.insert("src/main.rs".to_string());
        status.untracked.insert("new_file.rs".to_string());

        assert!(status.is_dirty("src/main.rs"));
        assert!(status.is_dirty("new_file.rs"));
        assert!(!status.is_dirty("other.rs"));
        assert_eq!(status.dirty_count(), 2);
    }

    #[test]
    fn test_git_status_normalize_path() {
        let mut status = GitStatus::default();
        status.modified.insert("src/main.rs".to_string());

        // Should handle ./ prefix
        assert!(status.is_dirty("./src/main.rs"));
        assert!(status.is_dirty("src/main.rs"));
    }
}
