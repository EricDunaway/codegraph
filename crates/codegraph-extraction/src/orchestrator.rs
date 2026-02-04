//! Extraction orchestrator - coordinates file scanning, parsing, and storage

use crate::error::ExtractionError;
use crate::extractor::ExtractorRegistry;
use crate::scanner::{FileScanner, ScannedFile};
use codegraph_types::{Config, ExtractionResult as TypesExtractionResult, Language};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Progress information for indexing operations
#[derive(Debug, Clone)]
pub struct IndexProgress {
    /// Current phase
    pub phase: IndexPhase,
    /// Current item number
    pub current: usize,
    /// Total items (may be 0 if unknown)
    pub total: usize,
    /// Current file being processed
    pub current_file: Option<String>,
}

/// Phases of the indexing process
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexPhase {
    /// Scanning filesystem for files
    Scanning,
    /// Parsing source files
    Parsing,
    /// Storing nodes and edges
    Storing,
    /// Resolving references
    Resolving,
}

/// Result of a full indexing operation
#[derive(Debug, Default)]
pub struct IndexResult {
    /// Whether indexing completed successfully
    pub success: bool,
    /// Number of files indexed
    pub files_indexed: usize,
    /// Number of files skipped
    pub files_skipped: usize,
    /// Number of nodes created
    pub nodes_created: usize,
    /// Number of edges created
    pub edges_created: usize,
    /// Errors encountered (non-fatal)
    pub errors: Vec<String>,
    /// Duration in milliseconds
    pub duration_ms: u64,
}

/// Result of a sync operation
#[derive(Debug, Default)]
pub struct SyncResult {
    /// Files checked for changes
    pub files_checked: usize,
    /// New files added
    pub files_added: usize,
    /// Modified files updated
    pub files_modified: usize,
    /// Deleted files removed
    pub files_removed: usize,
    /// Nodes updated
    pub nodes_updated: usize,
    /// Duration in milliseconds
    pub duration_ms: u64,
}

/// Extraction result for a single file
#[derive(Debug)]
pub struct FileExtractionResult {
    /// The scanned file info
    pub file: ScannedFile,
    /// Extraction result
    pub result: TypesExtractionResult,
}

/// Extraction orchestrator - coordinates the full extraction pipeline
pub struct ExtractionOrchestrator {
    root_dir: std::path::PathBuf,
    config: Config,
    scanner: FileScanner,
    extractors: ExtractorRegistry,
}

impl ExtractionOrchestrator {
    /// Create a new orchestrator
    pub fn new(root_dir: impl AsRef<Path>, config: Config) -> Result<Self, ExtractionError> {
        let root_dir = root_dir.as_ref().to_path_buf();
        let scanner = FileScanner::new(&root_dir, &config)?;
        let extractors = ExtractorRegistry::new();

        Ok(Self {
            root_dir,
            config,
            scanner,
            extractors,
        })
    }

    /// Create with default configuration
    pub fn with_defaults(root_dir: impl AsRef<Path>) -> Result<Self, ExtractionError> {
        Self::new(root_dir, Config::default())
    }

    /// Index all files in the project
    pub fn index_all<F>(&self, mut on_progress: F) -> Result<IndexResult, ExtractionError>
    where
        F: FnMut(IndexProgress),
    {
        let start = std::time::Instant::now();
        let mut result = IndexResult::default();

        // Phase 1: Scan for files
        on_progress(IndexProgress {
            phase: IndexPhase::Scanning,
            current: 0,
            total: 0,
            current_file: None,
        });

        let scan_result = self.scanner.scan_with_progress(|current, file| {
            on_progress(IndexProgress {
                phase: IndexPhase::Scanning,
                current,
                total: 0, // Unknown during scanning
                current_file: Some(file.to_string()),
            });
        })?;

        let total_files = scan_result.files.len();
        result.files_skipped = scan_result.skipped_files;

        // Phase 2: Parse files
        let mut all_results: Vec<FileExtractionResult> = Vec::new();

        for (idx, file) in scan_result.files.into_iter().enumerate() {
            on_progress(IndexProgress {
                phase: IndexPhase::Parsing,
                current: idx + 1,
                total: total_files,
                current_file: Some(file.path.clone()),
            });

            match self.extract_file(&file) {
                Ok(extraction_result) => {
                    result.nodes_created += extraction_result.nodes.len();
                    result.edges_created += extraction_result.edges.len();
                    result.files_indexed += 1;

                    all_results.push(FileExtractionResult {
                        file,
                        result: extraction_result,
                    });
                }
                Err(e) => {
                    result.errors.push(format!("{}: {}", file.path, e));
                }
            }
        }

        // Phase 3: Storing is handled by caller (database layer)
        on_progress(IndexProgress {
            phase: IndexPhase::Storing,
            current: result.files_indexed,
            total: result.files_indexed,
            current_file: None,
        });

        // Phase 4: Resolution is handled by resolution crate
        on_progress(IndexProgress {
            phase: IndexPhase::Resolving,
            current: 0,
            total: 0,
            current_file: None,
        });

        result.success = result.errors.is_empty();
        result.duration_ms = start.elapsed().as_millis() as u64;

        Ok(result)
    }

    /// Extract a single file
    pub fn extract_file(&self, file: &ScannedFile) -> Result<TypesExtractionResult, ExtractionError> {
        // Read file content
        let content = fs::read_to_string(&file.absolute_path)
            .map_err(|e| ExtractionError::file_read(&file.absolute_path, e.to_string()))?;

        // Get appropriate extractor
        self.extractors.extract(&content, &file.path, file.language)
    }

    /// Extract from source string directly
    pub fn extract_from_source(
        &self,
        source: &str,
        file_path: &str,
        language: Language,
    ) -> Result<TypesExtractionResult, ExtractionError> {
        self.extractors.extract(source, file_path, language)
    }

    /// Get files that have changed since a previous index
    pub fn get_changed_files(
        &self,
        previous_hashes: &HashMap<String, String>,
    ) -> Result<Vec<ScannedFile>, ExtractionError> {
        self.scanner.get_changed_files(previous_hashes)
    }

    /// Sync incremental changes
    pub fn sync<F>(
        &self,
        previous_hashes: &HashMap<String, String>,
        previous_paths: &[String],
        mut on_progress: F,
    ) -> Result<(SyncResult, Vec<FileExtractionResult>), ExtractionError>
    where
        F: FnMut(IndexProgress),
    {
        let start = std::time::Instant::now();
        let mut result = SyncResult::default();
        let mut extraction_results = Vec::new();

        // Find changed and new files
        let changed_files = self.get_changed_files(previous_hashes)?;
        let removed_files = self.scanner.get_removed_files(previous_paths)?;

        result.files_checked = previous_hashes.len();
        result.files_removed = removed_files.len();

        // Categorize changed files
        for file in &changed_files {
            if previous_hashes.contains_key(&file.path) {
                result.files_modified += 1;
            } else {
                result.files_added += 1;
            }
        }

        // Extract changed files
        let total = changed_files.len();
        for (idx, file) in changed_files.into_iter().enumerate() {
            on_progress(IndexProgress {
                phase: IndexPhase::Parsing,
                current: idx + 1,
                total,
                current_file: Some(file.path.clone()),
            });

            if let Ok(extraction_result) = self.extract_file(&file) {
                result.nodes_updated += extraction_result.nodes.len();
                extraction_results.push(FileExtractionResult {
                    file,
                    result: extraction_result,
                });
            }
        }

        result.duration_ms = start.elapsed().as_millis() as u64;

        Ok((result, extraction_results))
    }

    /// Get the root directory
    pub fn root_dir(&self) -> &Path {
        &self.root_dir
    }

    /// Get the configuration
    pub fn config(&self) -> &Config {
        &self.config
    }
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
    fn test_orchestrator_index_all() {
        let temp_dir = TempDir::new().unwrap();

        create_test_file(
            temp_dir.path(),
            "src/lib.rs",
            "pub fn hello() { println!(\"Hello\"); }",
        );
        create_test_file(
            temp_dir.path(),
            "src/main.rs",
            "fn main() { hello(); }",
        );

        let orchestrator = ExtractionOrchestrator::with_defaults(temp_dir.path()).unwrap();
        let result = orchestrator.index_all(|_| {}).unwrap();

        assert_eq!(result.files_indexed, 2);
        assert!(result.nodes_created >= 4); // At least file nodes + functions
        assert!(result.success);
    }

    #[test]
    fn test_extract_from_source() {
        let temp_dir = TempDir::new().unwrap();
        let orchestrator = ExtractionOrchestrator::with_defaults(temp_dir.path()).unwrap();

        let source = "pub fn test_function() {}";
        let result = orchestrator
            .extract_from_source(source, "test.rs", Language::Rust)
            .unwrap();

        assert!(!result.nodes.is_empty());
        let func = result.nodes.iter().find(|n| n.name == "test_function");
        assert!(func.is_some());
    }

    #[test]
    fn test_orchestrator_with_multiple_languages() {
        let temp_dir = TempDir::new().unwrap();

        create_test_file(temp_dir.path(), "lib.rs", "pub fn rust_func() {}");
        create_test_file(temp_dir.path(), "app.ts", "export function tsFunc() {}");
        create_test_file(temp_dir.path(), "script.py", "def py_func(): pass");

        let orchestrator = ExtractionOrchestrator::with_defaults(temp_dir.path()).unwrap();
        let result = orchestrator.index_all(|_| {}).unwrap();

        assert_eq!(result.files_indexed, 3);
        assert!(result.success);
    }
}
