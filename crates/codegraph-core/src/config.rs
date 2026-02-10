//! CodeGraph configuration

use codegraph_types::{EmbeddingTextConfig, EnrichmentConfig, LspConfig};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

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

/// JSON-serializable configuration file format
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConfigFile {
    /// Schema version for migrations
    #[serde(default = "default_version")]
    pub version: u32,

    /// LSP configuration
    #[serde(default)]
    pub lsp: LspConfig,

    /// Enrichment configuration
    #[serde(default)]
    pub enrichment: EnrichmentConfig,

    /// Embedding text configuration
    #[serde(default)]
    pub embedding: EmbeddingTextConfig,

    /// Additional exclude patterns
    #[serde(default)]
    pub exclude: Vec<String>,

    /// Maximum file size to index (in bytes)
    #[serde(default = "default_max_file_size")]
    pub max_file_size: usize,
}

fn default_version() -> u32 {
    1
}
fn default_max_file_size() -> usize {
    10 * 1024 * 1024
}

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

    /// LSP configuration
    pub lsp: LspConfig,

    /// Enrichment configuration
    pub enrichment: EnrichmentConfig,

    /// Embedding text configuration
    pub embedding: EmbeddingTextConfig,
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
            lsp: LspConfig::default(),
            enrichment: EnrichmentConfig::default(),
            embedding: EmbeddingTextConfig::default(),
        }
    }

    /// Load configuration from a project root directory
    ///
    /// Looks for `.codegraph/config.json` and merges with defaults.
    /// If no config file exists, returns defaults.
    pub fn load(root: &Path) -> Result<Self, std::io::Error> {
        let root = root.to_path_buf();
        let data_dir = root.join(".codegraph");
        let config_path = data_dir.join("config.json");

        let mut config = Self::new(root);

        if config_path.exists() {
            let json = fs::read_to_string(&config_path)?;
            let file_config: ConfigFile = serde_json::from_str(&json).map_err(|e| {
                std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string())
            })?;

            // Merge file config with defaults
            config.lsp = file_config.lsp;
            config.enrichment = file_config.enrichment;
            config.embedding = file_config.embedding;
            config.max_file_size = file_config.max_file_size;

            // Add extra exclude patterns from config file
            config.exclude_patterns.extend(file_config.exclude);
        }

        Ok(config)
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

    /// Set LSP configuration
    pub fn with_lsp(mut self, lsp: LspConfig) -> Self {
        self.lsp = lsp;
        self
    }

    /// Set enrichment configuration
    pub fn with_enrichment(mut self, enrichment: EnrichmentConfig) -> Self {
        self.enrichment = enrichment;
        self
    }

    /// Set embedding configuration
    pub fn with_embedding(mut self, embedding: EmbeddingTextConfig) -> Self {
        self.embedding = embedding;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codegraph_types::{LspScope, LspServerConfig};
    use tempfile::TempDir;

    #[test]
    fn test_default_config() {
        let config = CodeGraphConfig::new(PathBuf::from("/project"));
        assert_eq!(config.root, PathBuf::from("/project"));
        assert_eq!(config.data_dir, PathBuf::from("/project/.codegraph"));
        assert_eq!(
            config.db_path,
            PathBuf::from("/project/.codegraph/codegraph.db")
        );
        assert!(!config.exclude_patterns.is_empty());
    }

    #[test]
    fn test_exclude_patterns() {
        let config = CodeGraphConfig::new(PathBuf::from("/project"));
        assert!(config
            .exclude_patterns
            .contains(&"node_modules/**".to_string()));
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

    #[test]
    fn test_default_enrichment_config() {
        let config = CodeGraphConfig::new(PathBuf::from("/project"));
        assert!(!config.lsp.enabled);
        assert_eq!(config.enrichment.lsp_scope, LspScope::Hybrid);
        assert_eq!(config.embedding.max_tokens, 2000);
    }

    #[test]
    fn test_load_config_from_json() {
        let temp = TempDir::new().unwrap();
        let codegraph_dir = temp.path().join(".codegraph");
        fs::create_dir_all(&codegraph_dir).unwrap();

        let config_json = r#"{
            "version": 2,
            "lsp": {
                "enabled": true,
                "typescript": {
                    "enabled": true,
                    "server": "typescript-language-server",
                    "args": ["--stdio"]
                }
            },
            "enrichment": {
                "lsp_scope": "hybrid",
                "cascade_depth": 2
            },
            "embedding": {
                "max_tokens": 3000
            }
        }"#;

        fs::write(codegraph_dir.join("config.json"), config_json).unwrap();

        let config = CodeGraphConfig::load(temp.path()).unwrap();
        assert!(config.lsp.enabled);
        assert_eq!(config.enrichment.cascade_depth, 2);
        assert_eq!(config.embedding.max_tokens, 3000);

        // Check TypeScript LSP config
        let ts_config = config.lsp.typescript.unwrap();
        assert!(ts_config.enabled);
        assert_eq!(ts_config.server, "typescript-language-server");
    }

    #[test]
    fn test_load_config_defaults_when_missing() {
        let temp = TempDir::new().unwrap();
        // No config file

        let config = CodeGraphConfig::load(temp.path()).unwrap();
        assert!(!config.lsp.enabled);
        assert_eq!(config.enrichment.lsp_scope, LspScope::Hybrid);
        assert_eq!(config.embedding.max_tokens, 2000);
    }

    #[test]
    fn test_load_config_with_extra_exclude_patterns() {
        let temp = TempDir::new().unwrap();
        let codegraph_dir = temp.path().join(".codegraph");
        fs::create_dir_all(&codegraph_dir).unwrap();

        let config_json = r#"{
            "exclude": ["custom/**", "*.generated.ts"]
        }"#;

        fs::write(codegraph_dir.join("config.json"), config_json).unwrap();

        let config = CodeGraphConfig::load(temp.path()).unwrap();
        assert!(config.exclude_patterns.contains(&"custom/**".to_string()));
        assert!(config
            .exclude_patterns
            .contains(&"*.generated.ts".to_string()));
        // Should also have default patterns
        assert!(config
            .exclude_patterns
            .contains(&"node_modules/**".to_string()));
    }

    #[test]
    fn test_config_with_lsp_builder() {
        let lsp = LspConfig {
            enabled: true,
            typescript: Some(LspServerConfig {
                enabled: true,
                server: "tsserver".to_string(),
                args: vec![],
            }),
            ..Default::default()
        };

        let config = CodeGraphConfig::new(PathBuf::from("/project")).with_lsp(lsp);

        assert!(config.lsp.enabled);
        assert!(config.lsp.typescript.is_some());
    }
}
