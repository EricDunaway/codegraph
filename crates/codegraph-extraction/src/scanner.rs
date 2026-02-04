//! File scanning with glob pattern matching

use crate::error::ExtractionError;
use codegraph_types::{Config, Language, DEFAULT_EXCLUDE, DEFAULT_INCLUDE};
use globset::{Glob, GlobSet, GlobSetBuilder};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Result of scanning a directory
#[derive(Debug, Default)]
pub struct ScanResult {
    /// Files found matching include patterns
    pub files: Vec<ScannedFile>,
    /// Directories skipped due to exclude patterns
    pub skipped_dirs: usize,
    /// Files skipped due to exclude patterns
    pub skipped_files: usize,
}

/// A scanned file with metadata
#[derive(Debug, Clone)]
pub struct ScannedFile {
    /// Relative path from root
    pub path: String,
    /// Absolute path
    pub absolute_path: PathBuf,
    /// Detected language
    pub language: Language,
    /// File size in bytes
    pub size: u64,
    /// Content hash (SHA-256)
    pub content_hash: String,
}

/// File scanner with configurable patterns
pub struct FileScanner {
    root_dir: PathBuf,
    include_patterns: GlobSet,
    exclude_patterns: GlobSet,
    max_file_size: u64,
}

impl FileScanner {
    /// Create a new scanner with the given configuration
    pub fn new(root_dir: impl AsRef<Path>, config: &Config) -> Result<Self, ExtractionError> {
        let root_dir = root_dir.as_ref().to_path_buf();

        let include_patterns = build_glob_set(&config.include)?;
        let exclude_patterns = build_glob_set(&config.exclude)?;

        Ok(Self {
            root_dir,
            include_patterns,
            exclude_patterns,
            max_file_size: config.max_file_size,
        })
    }

    /// Create a scanner with default patterns
    pub fn with_defaults(root_dir: impl AsRef<Path>) -> Result<Self, ExtractionError> {
        let root_dir = root_dir.as_ref().to_path_buf();

        let include: Vec<String> = DEFAULT_INCLUDE.iter().map(|s| (*s).to_string()).collect();
        let exclude: Vec<String> = DEFAULT_EXCLUDE.iter().map(|s| (*s).to_string()).collect();

        let include_patterns = build_glob_set(&include)?;
        let exclude_patterns = build_glob_set(&exclude)?;

        Ok(Self {
            root_dir,
            include_patterns,
            exclude_patterns,
            max_file_size: 1024 * 1024, // 1MB default
        })
    }

    /// Scan the directory for source files
    pub fn scan(&self) -> Result<ScanResult, ExtractionError> {
        self.scan_with_progress(|_, _| {})
    }

    /// Scan with progress callback
    pub fn scan_with_progress<F>(&self, mut on_progress: F) -> Result<ScanResult, ExtractionError>
    where
        F: FnMut(usize, &str),
    {
        let mut result = ScanResult::default();
        let mut count = 0;

        for entry in WalkDir::new(&self.root_dir)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| !self.should_skip_entry(e))
        {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };

            if !entry.file_type().is_file() {
                continue;
            }

            let absolute_path = entry.path().to_path_buf();
            let relative_path = match absolute_path.strip_prefix(&self.root_dir) {
                Ok(p) => p.to_string_lossy().to_string(),
                Err(_) => continue,
            };

            // Check include patterns
            if !self.include_patterns.is_match(&relative_path) {
                result.skipped_files += 1;
                continue;
            }

            // Check exclude patterns
            if self.exclude_patterns.is_match(&relative_path) {
                result.skipped_files += 1;
                continue;
            }

            // Check file size
            let metadata = match fs::metadata(&absolute_path) {
                Ok(m) => m,
                Err(_) => continue,
            };

            if metadata.len() > self.max_file_size {
                result.skipped_files += 1;
                continue;
            }

            // Detect language
            let language = Language::from_extension(
                absolute_path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or(""),
            );

            if language == Language::Unknown {
                result.skipped_files += 1;
                continue;
            }

            // Calculate content hash
            let content_hash = match hash_file(&absolute_path) {
                Ok(h) => h,
                Err(_) => continue,
            };

            count += 1;
            on_progress(count, &relative_path);

            result.files.push(ScannedFile {
                path: relative_path,
                absolute_path,
                language,
                size: metadata.len(),
                content_hash,
            });
        }

        Ok(result)
    }

    /// Check if an entry should be skipped (for walkdir filter)
    fn should_skip_entry(&self, entry: &walkdir::DirEntry) -> bool {
        // Never skip the root directory itself
        if entry.depth() == 0 {
            return false;
        }

        let path = match entry.path().strip_prefix(&self.root_dir) {
            Ok(p) => p.to_string_lossy().to_string(),
            Err(_) => return false,
        };

        // Skip hidden directories (except .codegraph)
        if entry.file_type().is_dir() {
            if let Some(name) = entry.file_name().to_str() {
                if name.starts_with('.') && name != ".codegraph" {
                    return true;
                }
            }
        }

        // Check exclude patterns for directories
        if entry.file_type().is_dir() {
            let dir_path = format!("{}/", path);
            if self.exclude_patterns.is_match(&dir_path) || self.exclude_patterns.is_match(&path) {
                return true;
            }
        }

        false
    }

    /// Get files that have changed since last scan
    pub fn get_changed_files(
        &self,
        previous_hashes: &std::collections::HashMap<String, String>,
    ) -> Result<Vec<ScannedFile>, ExtractionError> {
        let scan_result = self.scan()?;

        let changed: Vec<ScannedFile> = scan_result
            .files
            .into_iter()
            .filter(|f| {
                match previous_hashes.get(&f.path) {
                    Some(prev_hash) => prev_hash != &f.content_hash,
                    None => true, // New file
                }
            })
            .collect();

        Ok(changed)
    }

    /// Get files that were removed since last scan
    pub fn get_removed_files(
        &self,
        previous_paths: &[String],
    ) -> Result<Vec<String>, ExtractionError> {
        let scan_result = self.scan()?;
        let current_paths: std::collections::HashSet<&str> =
            scan_result.files.iter().map(|f| f.path.as_str()).collect();

        let removed: Vec<String> = previous_paths
            .iter()
            .filter(|p| !current_paths.contains(p.as_str()))
            .cloned()
            .collect();

        Ok(removed)
    }
}

/// Build a GlobSet from patterns
fn build_glob_set(patterns: &[String]) -> Result<GlobSet, ExtractionError> {
    let mut builder = GlobSetBuilder::new();

    for pattern in patterns {
        // Ensure pattern works with relative paths (no leading ./)
        let normalized = pattern
            .trim_start_matches("./")
            .to_string();

        let glob = Glob::new(&normalized)
            .map_err(|e| ExtractionError::GlobPattern(format!("{pattern}: {e}")))?;
        builder.add(glob);
    }

    builder
        .build()
        .map_err(|e| ExtractionError::GlobPattern(e.to_string()))
}

/// Calculate SHA-256 hash of file contents
pub fn hash_file(path: &Path) -> Result<String, std::io::Error> {
    let content = fs::read(path)?;
    hash_content(&content)
}

/// Calculate SHA-256 hash of content
pub fn hash_content(content: &[u8]) -> Result<String, std::io::Error> {
    let mut hasher = Sha256::new();
    hasher.update(content);
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use tempfile::TempDir;

    fn create_test_file(dir: &Path, name: &str, content: &str) {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let mut file = File::create(path).unwrap();
        file.write_all(content.as_bytes()).unwrap();
    }

    #[test]
    fn test_scan_finds_rust_files() {
        let temp_dir = TempDir::new().unwrap();
        create_test_file(temp_dir.path(), "lib.rs", "fn main() {}");
        create_test_file(temp_dir.path(), "src/mod.rs", "pub mod test;");

        let scanner = FileScanner::with_defaults(temp_dir.path()).unwrap();
        let result = scanner.scan().unwrap();

        assert_eq!(result.files.len(), 2);
        assert!(result.files.iter().all(|f| f.language == Language::Rust));
    }

    #[test]
    fn test_scan_excludes_node_modules() {
        let temp_dir = TempDir::new().unwrap();
        create_test_file(temp_dir.path(), "src/index.ts", "export const x = 1;");
        create_test_file(temp_dir.path(), "node_modules/pkg/index.ts", "export const y = 2;");

        let scanner = FileScanner::with_defaults(temp_dir.path()).unwrap();
        let result = scanner.scan().unwrap();

        assert_eq!(result.files.len(), 1);
        assert_eq!(result.files[0].path, "src/index.ts");
    }

    #[test]
    fn test_scan_excludes_hidden_dirs() {
        let temp_dir = TempDir::new().unwrap();
        create_test_file(temp_dir.path(), "src/lib.rs", "fn test() {}");
        create_test_file(temp_dir.path(), ".git/config", "not a rust file");

        let scanner = FileScanner::with_defaults(temp_dir.path()).unwrap();
        let result = scanner.scan().unwrap();

        assert_eq!(result.files.len(), 1);
    }

    #[test]
    fn test_hash_content() {
        let hash1 = hash_content(b"hello world").unwrap();
        let hash2 = hash_content(b"hello world").unwrap();
        let hash3 = hash_content(b"different").unwrap();

        assert_eq!(hash1, hash2);
        assert_ne!(hash1, hash3);
        assert_eq!(hash1.len(), 64); // SHA-256 produces 64 hex chars
    }

    #[test]
    fn test_language_detection() {
        let temp_dir = TempDir::new().unwrap();
        create_test_file(temp_dir.path(), "app.ts", "const x = 1;");
        create_test_file(temp_dir.path(), "main.rs", "fn main() {}");
        create_test_file(temp_dir.path(), "script.py", "print('hello')");
        create_test_file(temp_dir.path(), "readme.txt", "not code");

        let scanner = FileScanner::with_defaults(temp_dir.path()).unwrap();
        let result = scanner.scan().unwrap();

        assert_eq!(result.files.len(), 3);

        let ts_file = result.files.iter().find(|f| f.path == "app.ts").unwrap();
        assert_eq!(ts_file.language, Language::TypeScript);

        let rs_file = result.files.iter().find(|f| f.path == "main.rs").unwrap();
        assert_eq!(rs_file.language, Language::Rust);

        let py_file = result.files.iter().find(|f| f.path == "script.py").unwrap();
        assert_eq!(py_file.language, Language::Python);
    }
}
