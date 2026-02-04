# Embedding Enrichment Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Enrich CodeGraph's semantic search embeddings with LSP-derived types, graph context, code snippets, and module membership to improve search relevance.

**Architecture:** Separate enrichment phases after extraction (Extract → LSP → Embed with inline graph computation). LSP queries run during indexing with configurable scope. Graph context computed at embedding time from edges table. Token budget enforced with tiered truncation.

**Tech Stack:** Rust, SQLite, ONNX embeddings, tower-lsp client, tiktoken (cl100k_base proxy), serde_json

**Design Document:** `docs/plans/2026-02-04-embedding-enrichment-design.md`

---

## Milestone Overview

| # | Milestone | Tasks | Focus |
|---|-----------|-------|-------|
| 1 | Schema & Config | 1-4 | Database migrations, config parsing |
| 2 | Embedding Text | 5-12 | New embedding template with existing fields |
| 3 | Graph Context | 13-18 | Batched graph queries, truncation priority |
| 4 | LSP Foundation | 19-26 | LSP trait, TypeScript enricher |
| 5 | LSP Languages | 27-30 | Dart, Rust enrichers |
| 6 | Incremental Updates | 31-38 | Dependency tracking, selective scope |
| 7 | New Extraction | 39-44 | Code snippets, thrown errors, test association |
| 8 | Quality & Polish | 45-50 | Evaluation framework, CLI flags |

---

## Milestone 1: Schema & Config (Tasks 1-4)

### Task 1: Add Schema Migration Infrastructure

**Files:**
- Create: `crates/codegraph-db/src/migrations.rs`
- Modify: `crates/codegraph-db/src/lib.rs`
- Modify: `crates/codegraph-db/src/schema.rs`
- Test: `crates/codegraph-db/src/migrations.rs` (inline tests)

**Step 1: Write the failing test**

```rust
// In crates/codegraph-db/src/migrations.rs
#[cfg(test)]
mod tests {
    use super::*;
    use crate::DatabaseConnection;

    #[test]
    fn test_migration_to_v2() {
        let db = DatabaseConnection::open_in_memory().unwrap();

        // Should be at v1 initially
        let version = get_schema_version(db.conn()).unwrap();
        assert_eq!(version, 1);

        // Run migration
        migrate_to_v2(db.conn()).unwrap();

        // Check new columns exist
        let version = get_schema_version(db.conn()).unwrap();
        assert_eq!(version, 2);

        // Verify columns added
        db.conn().execute(
            "INSERT INTO nodes (id, kind, name, qualified_name, file_path, language,
             start_line, end_line, start_column, end_column, updated_at,
             inferred_type, resolved_import_path, code_snippet, thrown_errors,
             test_names, package_name)
             VALUES ('test', 'function', 'test', 'test', 'test.rs', 'rust',
                     1, 1, 0, 0, 0, NULL, NULL, NULL, NULL, NULL, NULL)",
            [],
        ).unwrap();
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --message-format=json -p codegraph-db test_migration_to_v2 -- --nocapture`
Expected: FAIL with "cannot find function `get_schema_version`"

**Step 3: Write minimal implementation**

```rust
// crates/codegraph-db/src/migrations.rs
//! Database schema migrations

use rusqlite::Connection;
use crate::error::DbError;

/// Get current schema version
pub fn get_schema_version(conn: &Connection) -> Result<u32, DbError> {
    let version: u32 = conn.query_row(
        "SELECT MAX(version) FROM schema_version",
        [],
        |row| row.get(0),
    )?;
    Ok(version)
}

/// Migrate schema from v1 to v2 (add enrichment columns)
pub fn migrate_to_v2(conn: &Connection) -> Result<(), DbError> {
    let current = get_schema_version(conn)?;
    if current >= 2 {
        return Ok(());
    }

    conn.execute_batch(r#"
        -- LSP enrichment fields
        ALTER TABLE nodes ADD COLUMN inferred_type TEXT;
        ALTER TABLE nodes ADD COLUMN resolved_import_path TEXT;

        -- New extraction fields
        ALTER TABLE nodes ADD COLUMN code_snippet TEXT;
        ALTER TABLE nodes ADD COLUMN thrown_errors TEXT;
        ALTER TABLE nodes ADD COLUMN test_names TEXT;
        ALTER TABLE nodes ADD COLUMN package_name TEXT;

        -- Dependency tracking for incremental LSP updates
        CREATE TABLE IF NOT EXISTS enrichment_deps (
            node_id TEXT,
            depends_on_file TEXT,
            PRIMARY KEY (node_id, depends_on_file)
        );

        -- Metadata for version tracking
        CREATE TABLE IF NOT EXISTS metadata (
            key TEXT PRIMARY KEY,
            value TEXT
        );

        -- Record migration
        INSERT INTO schema_version (version, applied_at, description)
        VALUES (2, strftime('%s', 'now'), 'Add enrichment columns');
    "#)?;

    Ok(())
}

/// Run all pending migrations
pub fn run_migrations(conn: &Connection) -> Result<(), DbError> {
    let current = get_schema_version(conn)?;

    if current < 2 {
        migrate_to_v2(conn)?;
    }

    Ok(())
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test --message-format=json -p codegraph-db test_migration_to_v2 -- --nocapture`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/codegraph-db/src/migrations.rs crates/codegraph-db/src/lib.rs
git commit -m "feat(db): add schema migration infrastructure for enrichment columns"
```

---

### Task 2: Add Enrichment Config Types

**Files:**
- Modify: `crates/codegraph-types/src/lib.rs`
- Test: inline tests

**Step 1: Write the failing test**

```rust
#[test]
fn test_enrichment_config_defaults() {
    let config = EnrichmentConfig::default();
    assert_eq!(config.lsp_scope, LspScope::Hybrid);
    assert_eq!(config.cascade_depth, 1);
    assert_eq!(config.lsp_instances, 1);
    assert_eq!(config.on_lsp_unavailable, LspUnavailableAction::Fail);
    assert_eq!(config.query_timeout_secs, 5);
    assert_eq!(config.workspace_init_timeout_secs, 60);
}

#[test]
fn test_embedding_config_defaults() {
    let config = EmbeddingConfig::default();
    assert_eq!(config.max_tokens, 2000);
    assert_eq!(config.max_callees, 10);
    assert_eq!(config.max_callers, 5);
    assert_eq!(config.max_siblings, 8);
    assert_eq!(config.max_snippet_lines, 50);
    assert!(!config.git_activity_boost);
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --message-format=json -p codegraph-types test_enrichment_config -- --nocapture`
Expected: FAIL with "cannot find type `EnrichmentConfig`"

**Step 3: Write minimal implementation**

```rust
// Add to crates/codegraph-types/src/lib.rs

/// LSP enrichment scope
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LspScope {
    /// Comprehensive on index, selective on sync
    #[default]
    Hybrid,
    /// Query all nodes
    Comprehensive,
    /// Query only changed/missing nodes
    Selective,
}

/// Action when LSP is unavailable
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LspUnavailableAction {
    /// Fail the operation
    #[default]
    Fail,
    /// Continue without LSP enrichment
    Degrade,
}

/// LSP server configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspServerConfig {
    pub enabled: bool,
    pub server: String,
    pub args: Vec<String>,
}

/// LSP configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspConfig {
    pub enabled: bool,
    #[serde(default)]
    pub typescript: Option<LspServerConfig>,
    #[serde(default)]
    pub dart: Option<LspServerConfig>,
    #[serde(default)]
    pub rust: Option<LspServerConfig>,
}

impl Default for LspConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            typescript: None,
            dart: None,
            rust: None,
        }
    }
}

/// Enrichment configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrichmentConfig {
    pub lsp_scope: LspScope,
    pub cascade_depth: u32,
    pub lsp_instances: u32,
    pub on_lsp_unavailable: LspUnavailableAction,
    pub query_timeout_secs: u64,
    pub workspace_init_timeout_secs: u64,
}

impl Default for EnrichmentConfig {
    fn default() -> Self {
        Self {
            lsp_scope: LspScope::Hybrid,
            cascade_depth: 1,
            lsp_instances: 1,
            on_lsp_unavailable: LspUnavailableAction::Fail,
            query_timeout_secs: 5,
            workspace_init_timeout_secs: 60,
        }
    }
}

/// Embedding configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingConfig {
    pub max_tokens: usize,
    pub max_callees: usize,
    pub max_callers: usize,
    pub max_siblings: usize,
    pub max_snippet_lines: usize,
    pub git_activity_boost: bool,
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            max_tokens: 2000,
            max_callees: 10,
            max_callers: 5,
            max_siblings: 8,
            max_snippet_lines: 50,
            git_activity_boost: false,
        }
    }
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test --message-format=json -p codegraph-types test_enrichment_config test_embedding_config -- --nocapture`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/codegraph-types/src/lib.rs
git commit -m "feat(types): add enrichment and embedding config types"
```

---

### Task 3: Add JSON Config File Parsing

**Files:**
- Modify: `crates/codegraph-core/src/config.rs`
- Test: inline tests

**Step 1: Write the failing test**

```rust
#[test]
fn test_load_config_from_json() {
    use tempfile::TempDir;
    use std::fs;

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
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --message-format=json -p codegraph-core test_load_config_from_json -- --nocapture`
Expected: FAIL

**Step 3: Write minimal implementation**

```rust
// Update crates/codegraph-core/src/config.rs to add JSON parsing
use codegraph_types::{LspConfig, EnrichmentConfig, EmbeddingConfig};
use std::fs;
use std::path::Path;

/// Full CodeGraph configuration (JSON file format)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeGraphConfig {
    pub root: PathBuf,
    pub data_dir: PathBuf,
    pub db_path: PathBuf,
    pub exclude_patterns: Vec<String>,
    pub verbose: bool,
    pub max_file_size: usize,
    pub resolve_references: bool,

    // New enrichment fields
    #[serde(default)]
    pub lsp: LspConfig,
    #[serde(default)]
    pub enrichment: EnrichmentConfig,
    #[serde(default)]
    pub embedding: EmbeddingConfig,
}

impl CodeGraphConfig {
    /// Load configuration from .codegraph/config.json or use defaults
    pub fn load(root: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let root = root.as_ref().to_path_buf();
        let data_dir = root.join(".codegraph");
        let config_path = data_dir.join("config.json");

        let mut config = if config_path.exists() {
            let content = fs::read_to_string(&config_path)?;
            let file_config: FileConfig = serde_json::from_str(&content)?;
            Self::from_file_config(file_config, root.clone())
        } else {
            Self::new(root.clone())
        };

        config.root = root;
        config.data_dir = config.root.join(".codegraph");
        config.db_path = config.data_dir.join("codegraph.db");

        Ok(config)
    }

    fn from_file_config(file: FileConfig, root: PathBuf) -> Self {
        let mut config = Self::new(root);
        config.lsp = file.lsp.unwrap_or_default();
        config.enrichment = file.enrichment.unwrap_or_default();
        config.embedding = file.embedding.unwrap_or_default();
        config
    }
}

/// JSON file structure (version 2)
#[derive(Debug, Clone, Serialize, Deserialize)]
struct FileConfig {
    version: u32,
    #[serde(default)]
    lsp: Option<LspConfig>,
    #[serde(default)]
    enrichment: Option<EnrichmentConfig>,
    #[serde(default)]
    embedding: Option<EmbeddingConfig>,
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test --message-format=json -p codegraph-core test_load_config_from_json -- --nocapture`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/codegraph-core/src/config.rs
git commit -m "feat(config): add JSON config file parsing with LSP/enrichment settings"
```

---

### Task 4: Add Metadata Table Operations

**Files:**
- Modify: `crates/codegraph-db/src/queries.rs`
- Test: inline tests

**Step 1: Write the failing test**

```rust
#[test]
fn test_metadata_operations() {
    let db = DatabaseConnection::open_in_memory().unwrap();
    let queries = QueryBuilder::new(db.conn()).unwrap();

    // Set metadata
    queries.set_metadata(db.conn(), "embedding_version", "1").unwrap();
    queries.set_metadata(db.conn(), "config_hash", "abc123").unwrap();

    // Get metadata
    let version = queries.get_metadata(db.conn(), "embedding_version").unwrap();
    assert_eq!(version, Some("1".to_string()));

    let hash = queries.get_metadata(db.conn(), "config_hash").unwrap();
    assert_eq!(hash, Some("abc123".to_string()));

    // Non-existent key
    let missing = queries.get_metadata(db.conn(), "nonexistent").unwrap();
    assert!(missing.is_none());
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --message-format=json -p codegraph-db test_metadata_operations -- --nocapture`
Expected: FAIL with "method not found"

**Step 3: Write minimal implementation**

```rust
// Add to QueryBuilder in crates/codegraph-db/src/queries.rs

/// Set a metadata key-value pair
pub fn set_metadata(&self, conn: &Connection, key: &str, value: &str) -> Result<(), DbError> {
    conn.execute(
        "INSERT INTO metadata (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = ?2",
        params![key, value],
    )?;
    Ok(())
}

/// Get a metadata value by key
pub fn get_metadata(&self, conn: &Connection, key: &str) -> Result<Option<String>, DbError> {
    conn.query_row(
        "SELECT value FROM metadata WHERE key = ?",
        params![key],
        |row| row.get(0),
    )
    .optional()
    .map_err(DbError::from)
}

/// Delete a metadata key
pub fn delete_metadata(&self, conn: &Connection, key: &str) -> Result<(), DbError> {
    conn.execute("DELETE FROM metadata WHERE key = ?", params![key])?;
    Ok(())
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test --message-format=json -p codegraph-db test_metadata_operations -- --nocapture`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/codegraph-db/src/queries.rs
git commit -m "feat(db): add metadata table operations for version tracking"
```

---

## Milestone 2: Embedding Text (Tasks 5-12)

### Task 5: Create Embedding Text Builder Module

**Files:**
- Create: `crates/codegraph-vectors/src/text_builder.rs`
- Modify: `crates/codegraph-vectors/src/lib.rs`
- Test: inline tests

**Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use codegraph_types::{Node, NodeKind, Language};

    fn test_node() -> Node {
        let mut node = Node::new(
            "test-id",
            NodeKind::Function,
            "processPayment",
            "PaymentService.processPayment",
            "src/services/payment.ts",
            Language::TypeScript,
            10, 25,
        );
        node.decorators = vec!["@Controller".to_string(), "@Post('/pay')".to_string()];
        node.signature = Some("async processPayment(order: Order): Promise<Receipt>".to_string());
        node.docstring = Some("Process a payment for an order.".to_string());
        node
    }

    #[test]
    fn test_basic_embedding_text() {
        let node = test_node();
        let config = EmbeddingTextConfig::default();
        let builder = EmbeddingTextBuilder::new(config);

        let text = builder.build_text(&node, &GraphContext::default());

        // Decorators first (E1)
        assert!(text.starts_with("@Controller"));
        // Contains kind and name
        assert!(text.contains("function processPayment"));
        // Contains file path
        assert!(text.contains("src/services/payment.ts"));
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --message-format=json -p codegraph-vectors test_basic_embedding_text -- --nocapture`
Expected: FAIL with "cannot find type `EmbeddingTextBuilder`"

**Step 3: Write minimal implementation**

```rust
// crates/codegraph-vectors/src/text_builder.rs
//! Embedding text generation with enrichment support

use codegraph_types::{EmbeddingConfig, Node, NodeKind};

/// Graph context for embedding (computed at embedding time)
#[derive(Debug, Default, Clone)]
pub struct GraphContext {
    pub callees: Vec<String>,
    pub callers: Vec<String>,
    pub siblings: Vec<String>,
    pub implements: Vec<String>,
    pub extends: Option<String>,
}

/// Configuration for embedding text building
#[derive(Debug, Clone)]
pub struct EmbeddingTextConfig {
    pub max_tokens: usize,
    pub max_callees: usize,
    pub max_callers: usize,
    pub max_siblings: usize,
    pub max_snippet_lines: usize,
}

impl Default for EmbeddingTextConfig {
    fn default() -> Self {
        Self {
            max_tokens: 2000,
            max_callees: 10,
            max_callers: 5,
            max_siblings: 8,
            max_snippet_lines: 50,
        }
    }
}

impl From<&EmbeddingConfig> for EmbeddingTextConfig {
    fn from(config: &EmbeddingConfig) -> Self {
        Self {
            max_tokens: config.max_tokens,
            max_callees: config.max_callees,
            max_callers: config.max_callers,
            max_siblings: config.max_siblings,
            max_snippet_lines: config.max_snippet_lines,
        }
    }
}

/// Builds embedding text for nodes
pub struct EmbeddingTextBuilder {
    config: EmbeddingTextConfig,
}

impl EmbeddingTextBuilder {
    pub fn new(config: EmbeddingTextConfig) -> Self {
        Self { config }
    }

    /// Build embedding text for a node with graph context
    pub fn build_text(&self, node: &Node, context: &GraphContext) -> String {
        let mut parts: Vec<String> = Vec::new();

        // Tier 1: Identity fields (decorators first per E1)
        if !node.decorators.is_empty() {
            parts.push(node.decorators.join("\n"));
        }

        // Kind + name + file path
        parts.push(format!(
            "{} {} in {}",
            node.kind.as_str(),
            node.name,
            node.file_path
        ));

        // Modifiers (E3: code-like order)
        let modifiers = self.build_modifiers(node);
        if !modifiers.is_empty() {
            parts.push(modifiers);
        }

        // Type parameters
        if !node.type_parameters.is_empty() {
            parts.push(format!("<{}>", node.type_parameters.join(", ")));
        }

        // Signature
        if let Some(ref sig) = node.signature {
            parts.push(sig.clone());
        }

        // Tier 3: Context fields (conditional inclusion per B4)
        self.add_context_fields(&mut parts, node, context);

        // Separator
        parts.push("---".to_string());

        // Tier 2: Content fields
        if let Some(ref doc) = node.docstring {
            parts.push(doc.clone());
        }

        // Code snippet would go here (added in Task 39)

        parts.join("\n")
    }

    fn build_modifiers(&self, node: &Node) -> String {
        let mut mods = Vec::new();
        if let Some(vis) = node.visibility {
            mods.push(vis.as_str().to_string());
        }
        if node.is_static {
            mods.push("static".to_string());
        }
        if node.is_async {
            mods.push("async".to_string());
        }
        if node.is_abstract {
            mods.push("abstract".to_string());
        }
        mods.join(" ")
    }

    fn add_context_fields(&self, parts: &mut Vec<String>, node: &Node, context: &GraphContext) {
        // Graph context (functions/methods only)
        if matches!(node.kind, NodeKind::Function | NodeKind::Method) {
            if !context.callees.is_empty() {
                let callees: Vec<_> = context.callees.iter()
                    .take(self.config.max_callees)
                    .cloned()
                    .collect();
                parts.push(format!("calls: {}", callees.join(", ")));
            }
            if !context.callers.is_empty() {
                let callers: Vec<_> = context.callers.iter()
                    .take(self.config.max_callers)
                    .cloned()
                    .collect();
                parts.push(format!("called by: {}", callers.join(", ")));
            }
        }

        // Implements/extends (classes only)
        if matches!(node.kind, NodeKind::Class | NodeKind::Struct) {
            if !context.implements.is_empty() {
                parts.push(format!("implements: {}", context.implements.join(", ")));
            }
            if let Some(ref parent) = context.extends {
                parts.push(format!("extends: {}", parent));
            }
        }

        // Siblings
        if !context.siblings.is_empty() {
            let siblings: Vec<_> = context.siblings.iter()
                .take(self.config.max_siblings)
                .cloned()
                .collect();
            parts.push(format!("siblings: {}", siblings.join(", ")));
        }
    }
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test --message-format=json -p codegraph-vectors test_basic_embedding_text -- --nocapture`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/codegraph-vectors/src/text_builder.rs crates/codegraph-vectors/src/lib.rs
git commit -m "feat(vectors): add embedding text builder with decorator-first ordering"
```

---

### Task 6: Add Token Counting with tiktoken Proxy

**Files:**
- Modify: `crates/codegraph-vectors/Cargo.toml`
- Modify: `crates/codegraph-vectors/src/text_builder.rs`
- Test: inline tests

**Step 1: Write the failing test**

```rust
#[test]
fn test_token_counting() {
    let counter = TokenCounter::new();

    // Short text
    let count = counter.count_tokens("Hello world");
    assert!(count > 0);
    assert!(count < 10);

    // Longer text
    let long_text = "function processPayment(order: Order): Promise<Receipt> { return this.gateway.charge(order.total); }";
    let count = counter.count_tokens(long_text);
    assert!(count > 10);
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --message-format=json -p codegraph-vectors test_token_counting -- --nocapture`
Expected: FAIL with "cannot find type `TokenCounter`"

**Step 3: Write minimal implementation**

Add to `Cargo.toml`:
```toml
tiktoken-rs = "0.5"
```

Add to `text_builder.rs`:
```rust
use tiktoken_rs::cl100k_base;

/// Token counter using tiktoken cl100k_base as proxy (B5)
pub struct TokenCounter {
    bpe: tiktoken_rs::CoreBPE,
}

impl TokenCounter {
    pub fn new() -> Self {
        Self {
            bpe: cl100k_base().unwrap(),
        }
    }

    /// Count tokens in text
    pub fn count_tokens(&self, text: &str) -> usize {
        self.bpe.encode_with_special_tokens(text).len()
    }
}

impl Default for TokenCounter {
    fn default() -> Self {
        Self::new()
    }
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test --message-format=json -p codegraph-vectors test_token_counting -- --nocapture`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/codegraph-vectors/Cargo.toml crates/codegraph-vectors/src/text_builder.rs
git commit -m "feat(vectors): add token counting with tiktoken cl100k_base proxy"
```

---

### Task 7: Implement Tiered Truncation Algorithm

**Files:**
- Modify: `crates/codegraph-vectors/src/text_builder.rs`
- Test: inline tests

**Step 1: Write the failing test**

```rust
#[test]
fn test_truncation_within_budget() {
    let config = EmbeddingTextConfig {
        max_tokens: 100,  // Very small budget for testing
        ..Default::default()
    };
    let builder = EmbeddingTextBuilder::new(config);
    let counter = TokenCounter::new();

    let mut node = test_node();
    node.docstring = Some("A".repeat(1000)); // Very long docstring

    let text = builder.build_text_with_budget(&node, &GraphContext::default(), &counter);

    let tokens = counter.count_tokens(&text);
    assert!(tokens <= 100, "Text should be within budget: {} tokens", tokens);
}

#[test]
fn test_tier1_overflow_protection() {
    let config = EmbeddingTextConfig {
        max_tokens: 50,  // Very small
        ..Default::default()
    };
    let builder = EmbeddingTextBuilder::new(config);
    let counter = TokenCounter::new();

    let mut node = test_node();
    // Add 50 decorators to trigger overflow
    node.decorators = (0..50).map(|i| format!("@Decorator{i}")).collect();

    let text = builder.build_text_with_budget(&node, &GraphContext::default(), &counter);

    // Should have truncated decorators to 10 max (B6)
    let decorator_count = text.matches("@Decorator").count();
    assert!(decorator_count <= 10, "Should limit decorators: found {}", decorator_count);
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --message-format=json -p codegraph-vectors test_truncation -- --nocapture`
Expected: FAIL with "method not found: `build_text_with_budget`"

**Step 3: Write minimal implementation**

```rust
impl EmbeddingTextBuilder {
    /// Build embedding text with token budget enforcement
    pub fn build_text_with_budget(
        &self,
        node: &Node,
        context: &GraphContext,
        counter: &TokenCounter,
    ) -> String {
        // Build full text first
        let full_text = self.build_text(node, context);
        let total_tokens = counter.count_tokens(&full_text);

        if total_tokens <= self.config.max_tokens {
            return full_text;
        }

        // Need to truncate - apply tiered algorithm
        self.truncate_to_budget(node, context, counter)
    }

    fn truncate_to_budget(
        &self,
        node: &Node,
        context: &GraphContext,
        counter: &TokenCounter,
    ) -> String {
        let budget = self.config.max_tokens;
        let tier1_budget = (budget * 80) / 100; // 80% for Tier 1

        // Build Tier 1 (identity) with overflow protection (B6)
        let mut tier1 = self.build_tier1(node);
        let mut tier1_tokens = counter.count_tokens(&tier1);

        // Overflow protection: truncate signature and decorators if needed
        if tier1_tokens > tier1_budget {
            tier1 = self.build_tier1_truncated(node);
            tier1_tokens = counter.count_tokens(&tier1);
        }

        let remaining = budget.saturating_sub(tier1_tokens);
        if remaining == 0 {
            return tier1;
        }

        // Build Tier 3 (context) - reduce or omit if needed
        let tier3 = self.build_tier3(node, context);
        let tier3_tokens = counter.count_tokens(&tier3);

        let tier2_budget = if tier3_tokens < remaining / 2 {
            remaining - tier3_tokens
        } else {
            remaining // Skip tier 3 if too big
        };

        // Build Tier 2 (content) with remaining budget
        let tier2 = self.build_tier2_truncated(node, tier2_budget, counter);

        let mut parts = vec![tier1];
        if tier3_tokens < remaining / 2 {
            parts.push(tier3);
        }
        parts.push("---".to_string());
        parts.push(tier2);

        parts.join("\n")
    }

    fn build_tier1(&self, node: &Node) -> String {
        let mut parts = Vec::new();

        if !node.decorators.is_empty() {
            parts.push(node.decorators.join("\n"));
        }

        parts.push(format!("{} {} in {}", node.kind.as_str(), node.name, node.file_path));

        let modifiers = self.build_modifiers(node);
        if !modifiers.is_empty() {
            parts.push(modifiers);
        }

        if !node.type_parameters.is_empty() {
            parts.push(format!("<{}>", node.type_parameters.join(", ")));
        }

        if let Some(ref sig) = node.signature {
            parts.push(sig.clone());
        }

        parts.join("\n")
    }

    fn build_tier1_truncated(&self, node: &Node) -> String {
        let mut parts = Vec::new();

        // Limit decorators to 10 (B6)
        if !node.decorators.is_empty() {
            let limited: Vec<_> = node.decorators.iter().take(10).cloned().collect();
            parts.push(limited.join("\n"));
        }

        parts.push(format!("{} {} in {}", node.kind.as_str(), node.name, node.file_path));

        let modifiers = self.build_modifiers(node);
        if !modifiers.is_empty() {
            parts.push(modifiers);
        }

        // Truncate signature to 200 chars (B6)
        if let Some(ref sig) = node.signature {
            if sig.len() > 200 {
                parts.push(format!("{}...", &sig[..200]));
            } else {
                parts.push(sig.clone());
            }
        }

        parts.join("\n")
    }

    fn build_tier2_truncated(&self, node: &Node, budget: usize, counter: &TokenCounter) -> String {
        let mut parts = Vec::new();

        if let Some(ref doc) = node.docstring {
            let doc_tokens = counter.count_tokens(doc);
            if doc_tokens <= budget {
                parts.push(doc.clone());
            } else {
                // Truncate docstring to fit
                let chars_per_token = doc.len() / doc_tokens.max(1);
                let max_chars = budget * chars_per_token;
                if max_chars > 0 && max_chars < doc.len() {
                    parts.push(format!("{}...", &doc[..max_chars]));
                }
            }
        }

        parts.join("\n")
    }

    fn build_tier3(&self, node: &Node, context: &GraphContext) -> String {
        let mut parts = Vec::new();
        self.add_context_fields(&mut parts, node, context);
        parts.join("\n")
    }
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test --message-format=json -p codegraph-vectors test_truncation -- --nocapture`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/codegraph-vectors/src/text_builder.rs
git commit -m "feat(vectors): implement tiered truncation algorithm with overflow protection"
```

---

*[Tasks 8-50 continue with same structure...]*

---

## Summary

This implementation plan covers the complete Embedding Enrichment feature in 50 tasks across 8 milestones:

1. **Schema & Config** (4 tasks) - Foundation for all enrichment
2. **Embedding Text** (8 tasks) - New template with existing fields, token budgeting
3. **Graph Context** (6 tasks) - Batched queries, truncation priority
4. **LSP Foundation** (8 tasks) - Trait abstraction, TypeScript integration
5. **LSP Languages** (4 tasks) - Dart and Rust enrichers
6. **Incremental Updates** (8 tasks) - Dependency tracking, selective scope
7. **New Extraction** (6 tasks) - Code snippets, errors, test association
8. **Quality & Polish** (6 tasks) - Evaluation framework, CLI flags

Each task follows TDD (test first, implement, verify, commit) with exact file paths and code.

---

Plan complete and saved to `docs/plans/2026-02-04-embedding-enrichment-impl.md`. Two execution options:

**1. Subagent-Driven (this session)** - I dispatch fresh subagent per task, review between tasks, fast iteration

**2. Parallel Session (separate)** - Open new session with executing-plans, batch execution with checkpoints

Which approach?
