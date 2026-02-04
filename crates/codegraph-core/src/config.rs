//! CodeGraph configuration

use std::path::PathBuf;

/// Default patterns to exclude from indexing
pub const DEFAULT_EXCLUDE_PATTERNS: &[&str] = &[
    // Version Control
    ".git/**",
    ".svn/**",
    ".hg/**",
    // Dependencies
    "node_modules/**",
    "vendor/**",
    "Pods/**",
    "packages/**",
    // Build Outputs
    "dist/**",
    "build/**",
    "out/**",
    "bin/**",
    "obj/**",
    "target/**",
    "_build/**",
    // JS/TS Frameworks
    ".next/**",
    ".nuxt/**",
    ".svelte-kit/**",
    ".vite/**",
    ".turbo/**",
    ".cache/**",
    "*.min.js",
    "*.bundle.js",
    "*.chunk.js",
    // React Native / Mobile
    ".expo/**",
    "ios/Pods/**",
    "android/build/**",
    "android/.gradle/**",
    // Python
    "__pycache__/**",
    ".venv/**",
    "venv/**",
    ".pytest_cache/**",
    ".mypy_cache/**",
    "*.egg-info/**",
    ".tox/**",
    // Go
    "go/pkg/mod/**",
    // Rust
    "target/debug/**",
    "target/release/**",
    // Java/Kotlin
    ".gradle/**",
    ".m2/**",
    "generated-sources/**",
    // C#/.NET
    ".vs/**",
    ".nuget/**",
    "artifacts/**",
    // C/C++
    "cmake-build-*/**",
    "CMakeFiles/**",
    "bazel-*/**",
    "Debug/**",
    "Release/**",
    // Swift/iOS
    "DerivedData/**",
    ".build/**",
    ".swiftpm/**",
    "Carthage/Build/**",
    // PHP
    ".composer/**",
    "storage/framework/**",
    // Ruby
    ".bundle/**",
    "tmp/cache/**",
    "public/assets/**",
    // Testing/Coverage
    "coverage/**",
    "htmlcov/**",
    ".nyc_output/**",
    "__snapshots__/**",
    // IDE/Editor
    ".idea/**",
    ".vscode/**",
    "*.sublime-*",
    // Documentation
    "docs/_build/**",
    "_site/**",
    // Logs
    "logs/**",
    "*.log",
    // CodeGraph internal
    ".codegraph/**",
];

/// CodeGraph configuration
#[derive(Debug, Clone)]
pub struct CodeGraphConfig {
    /// Root directory of the project
    pub root: PathBuf,

    /// Path to the .codegraph directory
    pub data_dir: PathBuf,

    /// Path to the database file
    pub db_path: PathBuf,

    /// Patterns to exclude from indexing
    pub exclude_patterns: Vec<String>,

    /// Whether to enable verbose logging
    pub verbose: bool,

    /// Maximum file size to index (in bytes)
    pub max_file_size: usize,

    /// Whether to resolve references after indexing
    pub resolve_references: bool,
}

impl CodeGraphConfig {
    /// Create a new configuration for the given root directory
    pub fn new(root: PathBuf) -> Self {
        let data_dir = root.join(".codegraph");
        let db_path = data_dir.join("codegraph.db");

        Self {
            root,
            data_dir,
            db_path,
            exclude_patterns: DEFAULT_EXCLUDE_PATTERNS
                .iter()
                .map(|s| s.to_string())
                .collect(),
            verbose: false,
            max_file_size: 10 * 1024 * 1024, // 10 MB
            resolve_references: true,
        }
    }

    /// Add additional exclude patterns
    pub fn with_exclude_patterns(mut self, patterns: Vec<String>) -> Self {
        self.exclude_patterns.extend(patterns);
        self
    }

    /// Set verbose logging
    pub fn with_verbose(mut self, verbose: bool) -> Self {
        self.verbose = verbose;
        self
    }

    /// Set maximum file size
    pub fn with_max_file_size(mut self, size: usize) -> Self {
        self.max_file_size = size;
        self
    }

    /// Set whether to resolve references
    pub fn with_resolve_references(mut self, resolve: bool) -> Self {
        self.resolve_references = resolve;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = CodeGraphConfig::new(PathBuf::from("/project"));
        assert_eq!(config.root, PathBuf::from("/project"));
        assert_eq!(config.data_dir, PathBuf::from("/project/.codegraph"));
        assert_eq!(config.db_path, PathBuf::from("/project/.codegraph/codegraph.db"));
        assert!(!config.exclude_patterns.is_empty());
    }

    #[test]
    fn test_exclude_patterns() {
        let config = CodeGraphConfig::new(PathBuf::from("/project"));
        assert!(config.exclude_patterns.contains(&"node_modules/**".to_string()));
        assert!(config.exclude_patterns.contains(&".git/**".to_string()));
        assert!(config.exclude_patterns.contains(&"target/**".to_string()));
    }

    #[test]
    fn test_config_builder() {
        let config = CodeGraphConfig::new(PathBuf::from("/project"))
            .with_verbose(true)
            .with_max_file_size(1024)
            .with_exclude_patterns(vec!["custom/**".to_string()]);

        assert!(config.verbose);
        assert_eq!(config.max_file_size, 1024);
        assert!(config.exclude_patterns.contains(&"custom/**".to_string()));
    }
}
