//! Git diff-based change detection for hook-triggered syncs.
//!
//! Uses `git diff --name-status -z` to identify changed files,
//! much faster than full filesystem hash scan for incremental updates.

use crate::change_detector::{compute_hash, ChangeKind, FileChange};
use crate::error::SyncError;
use codegraph_types::Language;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::process::Command;

/// Detect changes using git diff between checkpoint and HEAD.
///
/// This is faster than the full filesystem walk used by [`crate::change_detector::ChangeDetector`]
/// because it only examines files that git reports as changed, rather than hashing
/// every file in the repository.
pub struct GitDiffDetector<'a> {
    repo_root: &'a Path,
    excludes: &'a [String],
}

impl<'a> GitDiffDetector<'a> {
    pub fn new(repo_root: &'a Path, excludes: &'a [String]) -> Self {
        Self {
            repo_root,
            excludes,
        }
    }

    /// Detect changes since the given checkpoint commit.
    ///
    /// This merges three sources of changes:
    /// 1. Committed changes between `checkpoint` and HEAD
    /// 2. Working tree changes (unstaged modifications)
    /// 3. Untracked files
    ///
    /// Returns file changes suitable for `SyncManager` processing.
    /// Files are filtered to supported languages and excluded patterns.
    pub fn detect_changes(&self, checkpoint: &str) -> Result<Vec<FileChange>, SyncError> {
        // Get committed changes: checkpoint..HEAD
        let committed = self.git_diff_name_status(Some(checkpoint), "HEAD")?;

        // Get working tree changes: HEAD vs working tree
        let working = self.git_diff_name_status_working()?;

        // Get untracked files
        let untracked = self.git_ls_untracked()?;

        // Merge all changes, dedup by path (working tree wins over committed)
        let mut path_to_status: HashMap<String, ChangeKind> = HashMap::new();

        for (path, kind) in committed {
            path_to_status.insert(path, kind);
        }
        for (path, kind) in working {
            // Working tree state takes precedence over committed state
            path_to_status.insert(path, kind);
        }
        for path in untracked {
            path_to_status.entry(path).or_insert(ChangeKind::Added);
        }

        // Filter and classify
        let mut changes = Vec::new();
        for (path, kind) in path_to_status {
            // Skip excluded paths
            if self.is_excluded(&path) {
                continue;
            }

            // Determine language from extension
            let language = Language::from_extension(
                Path::new(&path)
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or(""),
            );

            // Skip unsupported languages
            if language == Language::Unknown {
                continue;
            }

            // Compute hash for non-deleted files
            let new_hash = if kind != ChangeKind::Deleted {
                let full_path = self.repo_root.join(&path);
                if full_path.exists() {
                    let content = fs::read_to_string(&full_path).map_err(SyncError::Io)?;
                    Some(compute_hash(&content))
                } else {
                    // File reported as changed but doesn't exist on disk; skip
                    continue;
                }
            } else {
                None
            };

            changes.push(FileChange {
                path,
                kind,
                language,
                old_hash: None,
                new_hash,
            });
        }

        Ok(changes)
    }

    /// Run `git diff --name-status -z` between two refs.
    fn git_diff_name_status(
        &self,
        from: Option<&str>,
        to: &str,
    ) -> Result<Vec<(String, ChangeKind)>, SyncError> {
        let mut cmd = Command::new("git");
        cmd.args(["diff", "--name-status", "-z"]);
        if let Some(from_ref) = from {
            cmd.arg(format!("{}..{}", from_ref, to));
        } else {
            cmd.arg(to);
        }
        cmd.current_dir(self.repo_root);

        let output = cmd.output().map_err(|e| SyncError::Git {
            message: format!("failed to run git diff: {}", e),
        })?;

        if !output.status.success() {
            return Err(SyncError::Git {
                message: format!(
                    "git diff failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                ),
            });
        }

        self.parse_name_status_z(&output.stdout)
    }

    /// Run `git diff --name-status -z HEAD` for working tree changes.
    fn git_diff_name_status_working(&self) -> Result<Vec<(String, ChangeKind)>, SyncError> {
        let output = Command::new("git")
            .args(["diff", "--name-status", "-z", "HEAD"])
            .current_dir(self.repo_root)
            .output()
            .map_err(|e| SyncError::Git {
                message: format!("failed to run git diff for working tree: {}", e),
            })?;

        if !output.status.success() {
            // No HEAD yet (initial commit scenario) — treat as empty
            return Ok(Vec::new());
        }

        self.parse_name_status_z(&output.stdout)
    }

    /// Run `git ls-files --others --exclude-standard -z` for untracked files.
    fn git_ls_untracked(&self) -> Result<Vec<String>, SyncError> {
        let output = Command::new("git")
            .args(["ls-files", "--others", "--exclude-standard", "-z"])
            .current_dir(self.repo_root)
            .output()
            .map_err(|e| SyncError::Git {
                message: format!("failed to run git ls-files: {}", e),
            })?;

        if !output.status.success() {
            return Ok(Vec::new());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(stdout
            .split('\0')
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect())
    }

    /// Parse NUL-delimited `--name-status` output.
    ///
    /// Format for simple statuses (A/M/D/T):
    ///   `STATUS\0PATH\0`
    ///
    /// Format for rename/copy (R/C):
    ///   `STATUS\0OLD_PATH\0NEW_PATH\0`
    ///
    /// Renames produce two entries: the new path as Added, and the old path as Deleted.
    /// Copies produce only the new path as Added.
    fn parse_name_status_z(&self, data: &[u8]) -> Result<Vec<(String, ChangeKind)>, SyncError> {
        let text = String::from_utf8_lossy(data);
        let parts: Vec<&str> = text.split('\0').collect();
        let mut result = Vec::new();
        let mut i = 0;

        while i < parts.len() {
            let status = parts[i];
            if status.is_empty() {
                i += 1;
                continue;
            }

            let first_char = match status.chars().next() {
                Some(c) => c,
                None => {
                    i += 1;
                    continue;
                }
            };

            match first_char {
                'A' => {
                    if i + 1 < parts.len() {
                        result.push((parts[i + 1].to_string(), ChangeKind::Added));
                        i += 2;
                    } else {
                        break;
                    }
                }
                'M' | 'T' => {
                    if i + 1 < parts.len() {
                        result.push((parts[i + 1].to_string(), ChangeKind::Modified));
                        i += 2;
                    } else {
                        break;
                    }
                }
                'D' => {
                    if i + 1 < parts.len() {
                        result.push((parts[i + 1].to_string(), ChangeKind::Deleted));
                        i += 2;
                    } else {
                        break;
                    }
                }
                'R' => {
                    // Rename: old path becomes Deleted, new path becomes Added
                    if i + 2 < parts.len() {
                        let old_path = parts[i + 1].to_string();
                        let new_path = parts[i + 2].to_string();
                        result.push((new_path, ChangeKind::Added));
                        result.push((old_path, ChangeKind::Deleted));
                        i += 3;
                    } else {
                        break;
                    }
                }
                'C' => {
                    // Copy: only new path as Added (source still exists)
                    if i + 2 < parts.len() {
                        let new_path = parts[i + 2].to_string();
                        result.push((new_path, ChangeKind::Added));
                        i += 3;
                    } else {
                        break;
                    }
                }
                _ => {
                    // Unknown status, skip
                    i += 1;
                }
            }
        }

        Ok(result)
    }

    /// Check if a path matches any exclude pattern.
    fn is_excluded(&self, path: &str) -> bool {
        self.excludes
            .iter()
            .any(|pattern| path.contains(pattern) || glob_match(pattern, path))
    }
}

/// Simple glob matching for exclude patterns.
///
/// Supports:
/// - `*.ext` — matches files ending with `.ext`
/// - `dir/` — matches paths starting with or containing the directory
/// - Exact or suffix matches for other patterns
fn glob_match(pattern: &str, path: &str) -> bool {
    if pattern.starts_with("*.") {
        // Extension match: *.rs matches src/main.rs
        path.ends_with(&pattern[1..])
    } else if pattern.ends_with('/') {
        // Directory match: node_modules/ matches node_modules/foo.js
        path.starts_with(pattern) || path.contains(&format!("/{}", pattern))
    } else {
        path == pattern || path.ends_with(&format!("/{}", pattern))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_name_status_simple() {
        let detector = GitDiffDetector::new(Path::new("/tmp"), &[]);
        let data = b"M\0src/main.rs\0A\0src/new.rs\0D\0old.rs\0";
        let result = detector.parse_name_status_z(data).unwrap();
        assert_eq!(result.len(), 3);
        assert_eq!(result[0], ("src/main.rs".to_string(), ChangeKind::Modified));
        assert_eq!(result[1], ("src/new.rs".to_string(), ChangeKind::Added));
        assert_eq!(result[2], ("old.rs".to_string(), ChangeKind::Deleted));
    }

    #[test]
    fn test_parse_rename() {
        let detector = GitDiffDetector::new(Path::new("/tmp"), &[]);
        let data = b"R100\0old_name.rs\0new_name.rs\0";
        let result = detector.parse_name_status_z(data).unwrap();
        // Rename produces: new_name Added + old_name Deleted
        assert!(result
            .iter()
            .any(|(p, k)| p == "new_name.rs" && *k == ChangeKind::Added));
        assert!(result
            .iter()
            .any(|(p, k)| p == "old_name.rs" && *k == ChangeKind::Deleted));
    }

    #[test]
    fn test_parse_copy() {
        let detector = GitDiffDetector::new(Path::new("/tmp"), &[]);
        let data = b"C100\0original.rs\0copy.rs\0";
        let result = detector.parse_name_status_z(data).unwrap();
        // Copy produces only new path as Added
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], ("copy.rs".to_string(), ChangeKind::Added));
    }

    #[test]
    fn test_parse_empty() {
        let detector = GitDiffDetector::new(Path::new("/tmp"), &[]);
        let result = detector.parse_name_status_z(b"").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_type_change() {
        let detector = GitDiffDetector::new(Path::new("/tmp"), &[]);
        // T status = file type changed (e.g. symlink to regular file)
        let data = b"T\0src/link.rs\0";
        let result = detector.parse_name_status_z(data).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], ("src/link.rs".to_string(), ChangeKind::Modified));
    }

    #[test]
    fn test_parse_mixed_statuses() {
        let detector = GitDiffDetector::new(Path::new("/tmp"), &[]);
        let data = b"A\0new.rs\0R090\0old.rs\0renamed.rs\0M\0changed.rs\0D\0gone.rs\0";
        let result = detector.parse_name_status_z(data).unwrap();
        assert_eq!(result.len(), 5);
        assert_eq!(result[0], ("new.rs".to_string(), ChangeKind::Added));
        assert_eq!(result[1], ("renamed.rs".to_string(), ChangeKind::Added));
        assert_eq!(result[2], ("old.rs".to_string(), ChangeKind::Deleted));
        assert_eq!(result[3], ("changed.rs".to_string(), ChangeKind::Modified));
        assert_eq!(result[4], ("gone.rs".to_string(), ChangeKind::Deleted));
    }

    #[test]
    fn test_glob_match_extension() {
        assert!(glob_match("*.rs", "src/main.rs"));
        assert!(glob_match("*.js", "app.js"));
        assert!(!glob_match("*.rs", "main.js"));
        assert!(glob_match("*.rs", "deep/nested/file.rs"));
    }

    #[test]
    fn test_glob_match_directory() {
        assert!(glob_match("node_modules/", "node_modules/foo.js"));
        assert!(glob_match("node_modules/", "src/node_modules/bar.js"));
        assert!(!glob_match("node_modules/", "my_node_modules.txt"));
    }

    #[test]
    fn test_glob_match_exact() {
        assert!(glob_match("Makefile", "Makefile"));
        assert!(glob_match("Makefile", "src/Makefile"));
        assert!(!glob_match("Makefile", "Makefile.bak"));
    }

    #[test]
    fn test_is_excluded() {
        let excludes = vec![
            "node_modules".to_string(),
            "*.lock".to_string(),
            "target/".to_string(),
        ];
        let detector = GitDiffDetector::new(Path::new("/tmp"), &excludes);

        assert!(detector.is_excluded("node_modules/foo.js"));
        assert!(detector.is_excluded("Cargo.lock"));
        assert!(detector.is_excluded("target/debug/main"));
        assert!(!detector.is_excluded("src/main.rs"));
    }
}
