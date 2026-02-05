# Embedding Enrichment Implementation Plan (Complete)

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
| 2 | Embedding Text | 5-12 | New embedding template with existing fields, token budgeting |
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
    migrations::run_migrations(db.conn()).unwrap();
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

### Task 8: Add LSP-Enriched Fields to Embedding Text

**Files:**
- Modify: `crates/codegraph-vectors/src/text_builder.rs`
- Test: inline tests

**Step 1: Write the failing test**

```rust
#[test]
fn test_embedding_with_lsp_fields() {
    let config = EmbeddingTextConfig::default();
    let builder = EmbeddingTextBuilder::new(config);

    let mut node = test_node();
    // Simulate LSP-enriched fields (will be added to Node struct later)
    let lsp_context = LspEnrichedContext {
        inferred_type: Some("Promise<Receipt>".to_string()),
        resolved_import_path: None,
    };

    let text = builder.build_text_with_lsp(&node, &GraphContext::default(), &lsp_context);

    assert!(text.contains("type: Promise<Receipt>"));
}

#[test]
fn test_import_with_resolved_path() {
    let config = EmbeddingTextConfig::default();
    let builder = EmbeddingTextBuilder::new(config);

    let node = Node::new(
        "import-id",
        NodeKind::Import,
        "PaymentService",
        "src/orders/service.ts::PaymentService",
        "src/orders/service.ts",
        Language::TypeScript,
        1, 1,
    );

    let lsp_context = LspEnrichedContext {
        inferred_type: None,
        resolved_import_path: Some("src/services/payment.ts".to_string()),
    };

    let text = builder.build_text_with_lsp(&node, &GraphContext::default(), &lsp_context);

    assert!(text.contains("resolves to: src/services/payment.ts"));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --message-format=json -p codegraph-vectors test_embedding_with_lsp -- --nocapture`
Expected: FAIL with "cannot find type `LspEnrichedContext`"

**Step 3: Write minimal implementation**

```rust
/// LSP-enriched context for a node
#[derive(Debug, Default, Clone)]
pub struct LspEnrichedContext {
    /// Inferred type from LSP hover
    pub inferred_type: Option<String>,
    /// Resolved import path from LSP definition
    pub resolved_import_path: Option<String>,
}

impl EmbeddingTextBuilder {
    /// Build embedding text with LSP-enriched fields
    pub fn build_text_with_lsp(
        &self,
        node: &Node,
        context: &GraphContext,
        lsp: &LspEnrichedContext,
    ) -> String {
        let mut parts: Vec<String> = Vec::new();

        // Tier 1: Identity fields (decorators first per E1)
        if !node.decorators.is_empty() {
            parts.push(node.decorators.join("\n"));
        }

        parts.push(format!(
            "{} {} in {}",
            node.kind.as_str(),
            node.name,
            node.file_path
        ));

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

        // LSP-enriched fields (B4: conditional inclusion)
        if let Some(ref inferred_type) = lsp.inferred_type {
            parts.push(format!("type: {}", inferred_type));
        }

        if let Some(ref resolved_path) = lsp.resolved_import_path {
            parts.push(format!("resolves to: {}", resolved_path));
        }

        // Tier 3: Context fields
        self.add_context_fields(&mut parts, node, context);

        // Separator
        parts.push("---".to_string());

        // Tier 2: Content fields
        if let Some(ref doc) = node.docstring {
            parts.push(doc.clone());
        }

        parts.join("\n")
    }
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test --message-format=json -p codegraph-vectors test_embedding_with_lsp -- --nocapture`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/codegraph-vectors/src/text_builder.rs
git commit -m "feat(vectors): add LSP-enriched fields (inferred_type, resolved_import_path) to embedding text"
```

---

### Task 9: Add Package/Module Field to Embedding Text

**Files:**
- Modify: `crates/codegraph-vectors/src/text_builder.rs`
- Test: inline tests

**Step 1: Write the failing test**

```rust
#[test]
fn test_package_name_in_embedding() {
    let config = EmbeddingTextConfig::default();
    let builder = EmbeddingTextBuilder::new(config);

    let mut node = test_node();
    let enrichment = NodeEnrichment {
        package_name: Some("@myapp/payments".to_string()),
        ..Default::default()
    };

    let text = builder.build_text_enriched(&node, &GraphContext::default(), &enrichment);

    assert!(text.contains("package: @myapp/payments"));
}

#[test]
fn test_package_fallback_to_directory() {
    let config = EmbeddingTextConfig::default();
    let builder = EmbeddingTextBuilder::new(config);

    let node = Node::new(
        "test-id",
        NodeKind::Function,
        "processPayment",
        "PaymentService.processPayment",
        "src/services/payment/handler.ts",
        Language::TypeScript,
        10, 25,
    );

    // No manifest-derived package, should fall back to directory path
    let enrichment = NodeEnrichment::default();

    let text = builder.build_text_enriched(&node, &GraphContext::default(), &enrichment);

    assert!(text.contains("package: src/services/payment"));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --message-format=json -p codegraph-vectors test_package -- --nocapture`
Expected: FAIL with "cannot find type `NodeEnrichment`"

**Step 3: Write minimal implementation**

```rust
/// All enrichment data for a node
#[derive(Debug, Default, Clone)]
pub struct NodeEnrichment {
    pub lsp: LspEnrichedContext,
    pub package_name: Option<String>,
    pub thrown_errors: Vec<String>,
    pub test_names: Vec<String>,
    pub code_snippet: Option<String>,
}

impl EmbeddingTextBuilder {
    /// Build embedding text with all enrichment
    pub fn build_text_enriched(
        &self,
        node: &Node,
        context: &GraphContext,
        enrichment: &NodeEnrichment,
    ) -> String {
        let mut parts: Vec<String> = Vec::new();

        // Tier 1: Identity fields
        if !node.decorators.is_empty() {
            parts.push(node.decorators.join("\n"));
        }

        parts.push(format!(
            "{} {} in {}",
            node.kind.as_str(),
            node.name,
            node.file_path
        ));

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

        // LSP fields
        if let Some(ref inferred_type) = enrichment.lsp.inferred_type {
            parts.push(format!("type: {}", inferred_type));
        }

        if let Some(ref resolved_path) = enrichment.lsp.resolved_import_path {
            parts.push(format!("resolves to: {}", resolved_path));
        }

        // Package name (M1-M3: prefer manifest, fallback to directory)
        let package = enrichment.package_name.clone()
            .unwrap_or_else(|| self.derive_package_from_path(&node.file_path));
        parts.push(format!("package: {}", package));

        // Graph context
        self.add_context_fields(&mut parts, node, context);

        // Separator
        parts.push("---".to_string());

        // Tier 2: Content fields
        if let Some(ref doc) = node.docstring {
            parts.push(doc.clone());
        }

        parts.join("\n")
    }

    /// Derive package name from file path (fallback when no manifest)
    fn derive_package_from_path(&self, file_path: &str) -> String {
        // Get parent directory, strip file name
        let path = std::path::Path::new(file_path);
        path.parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| ".".to_string())
    }
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test --message-format=json -p codegraph-vectors test_package -- --nocapture`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/codegraph-vectors/src/text_builder.rs
git commit -m "feat(vectors): add package/module field with manifest-first fallback to directory"
```

---

### Task 10: Add Thrown Errors and Test Names to Embedding

**Files:**
- Modify: `crates/codegraph-vectors/src/text_builder.rs`
- Test: inline tests

**Step 1: Write the failing test**

```rust
#[test]
fn test_thrown_errors_in_embedding() {
    let config = EmbeddingTextConfig::default();
    let builder = EmbeddingTextBuilder::new(config);

    let node = test_node();
    let enrichment = NodeEnrichment {
        thrown_errors: vec!["AuthenticationError".to_string(), "ValidationError".to_string()],
        ..Default::default()
    };

    let text = builder.build_text_enriched(&node, &GraphContext::default(), &enrichment);

    assert!(text.contains("throws: AuthenticationError, ValidationError"));
}

#[test]
fn test_test_names_in_embedding() {
    let config = EmbeddingTextConfig::default();
    let builder = EmbeddingTextBuilder::new(config);

    let node = test_node();
    let enrichment = NodeEnrichment {
        test_names: vec![
            "should handle expired cards".to_string(),
            "should reject invalid amounts".to_string(),
        ],
        ..Default::default()
    };

    let text = builder.build_text_enriched(&node, &GraphContext::default(), &enrichment);

    assert!(text.contains("tested by: should handle expired cards, should reject invalid amounts"));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --message-format=json -p codegraph-vectors test_thrown_errors test_test_names -- --nocapture`
Expected: FAIL (current implementation doesn't include these fields)

**Step 3: Write minimal implementation**

Update `build_text_enriched`:

```rust
impl EmbeddingTextBuilder {
    pub fn build_text_enriched(
        &self,
        node: &Node,
        context: &GraphContext,
        enrichment: &NodeEnrichment,
    ) -> String {
        let mut parts: Vec<String> = Vec::new();

        // ... existing Tier 1 code ...

        // Thrown errors (E8: functions/methods only)
        if matches!(node.kind, NodeKind::Function | NodeKind::Method) {
            if !enrichment.thrown_errors.is_empty() {
                parts.push(format!("throws: {}", enrichment.thrown_errors.join(", ")));
            }
        }

        // Test names (E7: compact test association)
        if !enrichment.test_names.is_empty() {
            parts.push(format!("tested by: {}", enrichment.test_names.join(", ")));
        }

        // Package name
        let package = enrichment.package_name.clone()
            .unwrap_or_else(|| self.derive_package_from_path(&node.file_path));
        parts.push(format!("package: {}", package));

        // Graph context
        self.add_context_fields(&mut parts, node, context);

        // ... existing Tier 2 code ...

        parts.join("\n")
    }
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test --message-format=json -p codegraph-vectors test_thrown_errors test_test_names -- --nocapture`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/codegraph-vectors/src/text_builder.rs
git commit -m "feat(vectors): add thrown errors and test names to embedding text"
```

---

### Task 11: Add Code Snippet Support to Embedding

**Files:**
- Modify: `crates/codegraph-vectors/src/text_builder.rs`
- Test: inline tests

**Step 1: Write the failing test**

```rust
#[test]
fn test_code_snippet_in_embedding() {
    let config = EmbeddingTextConfig::default();
    let builder = EmbeddingTextBuilder::new(config);

    let node = test_node();
    let enrichment = NodeEnrichment {
        code_snippet: Some("async processPayment(order: Order): Promise<Receipt> {\n  return this.gateway.charge(order.total);\n}".to_string()),
        ..Default::default()
    };

    let text = builder.build_text_enriched(&node, &GraphContext::default(), &enrichment);

    assert!(text.contains("return this.gateway.charge"));
}

#[test]
fn test_code_snippet_truncation() {
    let config = EmbeddingTextConfig {
        max_snippet_lines: 5,
        ..Default::default()
    };
    let builder = EmbeddingTextBuilder::new(config);

    let node = test_node();
    // Create a 20-line snippet
    let snippet = (0..20).map(|i| format!("line {}", i)).collect::<Vec<_>>().join("\n");
    let enrichment = NodeEnrichment {
        code_snippet: Some(snippet),
        ..Default::default()
    };

    let text = builder.build_text_enriched(&node, &GraphContext::default(), &enrichment);

    // Should only have 5 lines
    let snippet_lines = text.lines()
        .skip_while(|l| !l.starts_with("line "))
        .take_while(|l| l.starts_with("line "))
        .count();
    assert_eq!(snippet_lines, 5, "Should truncate to max_snippet_lines");
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --message-format=json -p codegraph-vectors test_code_snippet -- --nocapture`
Expected: FAIL

**Step 3: Write minimal implementation**

```rust
impl EmbeddingTextBuilder {
    pub fn build_text_enriched(
        &self,
        node: &Node,
        context: &GraphContext,
        enrichment: &NodeEnrichment,
    ) -> String {
        // ... existing code ...

        // Separator
        parts.push("---".to_string());

        // Tier 2: Content fields
        if let Some(ref doc) = node.docstring {
            parts.push(doc.clone());
        }

        // Code snippet (E4: adaptive up to max_snippet_lines)
        if let Some(ref snippet) = enrichment.code_snippet {
            let truncated = self.truncate_snippet(snippet);
            parts.push(truncated);
        }

        parts.join("\n")
    }

    /// Truncate code snippet to max lines
    fn truncate_snippet(&self, snippet: &str) -> String {
        let lines: Vec<&str> = snippet.lines().collect();
        if lines.len() <= self.config.max_snippet_lines {
            snippet.to_string()
        } else {
            let truncated: Vec<&str> = lines.into_iter()
                .take(self.config.max_snippet_lines)
                .collect();
            format!("{}...", truncated.join("\n"))
        }
    }
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test --message-format=json -p codegraph-vectors test_code_snippet -- --nocapture`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/codegraph-vectors/src/text_builder.rs
git commit -m "feat(vectors): add code snippet support with adaptive truncation"
```

---

### Task 12: Integrate Text Builder with Embedder

**Files:**
- Modify: `crates/codegraph-vectors/src/embedder.rs`
- Modify: `crates/codegraph-vectors/src/lib.rs`
- Test: integration test

**Step 1: Write the failing test**

```rust
#[test]
fn test_embed_node_with_enrichment() {
    let mut embedder = TextEmbedder::new(EmbedderConfig::default());
    let text_builder = EmbeddingTextBuilder::new(EmbeddingTextConfig::default());

    let node = test_node();
    let context = GraphContext::default();
    let enrichment = NodeEnrichment::default();

    // Build embedding text
    let text = text_builder.build_text_enriched(&node, &context, &enrichment);

    // Embed it
    let embedding = embedder.embed(&text).unwrap();

    assert_eq!(embedding.len(), 768); // nomic dimension
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --message-format=json -p codegraph-vectors test_embed_node_with_enrichment -- --nocapture`
Expected: FAIL or PASS depending on current embedder state

**Step 3: Write minimal implementation**

Update `lib.rs` to re-export:
```rust
pub mod text_builder;
pub use text_builder::{
    EmbeddingTextBuilder,
    EmbeddingTextConfig,
    GraphContext,
    LspEnrichedContext,
    NodeEnrichment,
    TokenCounter,
};
```

**Step 4: Run test to verify it passes**

Run: `cargo test --message-format=json -p codegraph-vectors test_embed_node_with_enrichment -- --nocapture`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/codegraph-vectors/src/lib.rs crates/codegraph-vectors/src/embedder.rs
git commit -m "feat(vectors): integrate text builder with embedder, export public API"
```

---

## Milestone 3: Graph Context (Tasks 13-18)

### Task 13: Add Graph Context Queries to GraphTraverser

**Files:**
- Modify: `crates/codegraph-graph/src/traverser.rs`
- Test: inline tests

**Step 1: Write the failing test**

```rust
#[test]
fn test_get_callees_for_embedding() {
    let db = setup_test_graph();
    let mut queries = QueryBuilder::new(db.conn()).unwrap();
    let mut traverser = GraphTraverser::new(db.conn(), &mut queries);

    // Assuming we have a node "fn_a" that calls "fn_b" and "fn_c"
    let callees = traverser.get_callees_for_embedding("fn_a", 10).unwrap();

    assert!(!callees.is_empty());
    // Should return node names, not full nodes
    assert!(callees.iter().all(|name| !name.is_empty()));
}

#[test]
fn test_get_callers_for_embedding() {
    let db = setup_test_graph();
    let mut queries = QueryBuilder::new(db.conn()).unwrap();
    let mut traverser = GraphTraverser::new(db.conn(), &mut queries);

    let callers = traverser.get_callers_for_embedding("fn_b", 5).unwrap();

    // fn_a calls fn_b, so fn_a should be in callers
    assert!(callers.contains(&"fn_a".to_string()));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --message-format=json -p codegraph-graph test_get_callees_for_embedding -- --nocapture`
Expected: FAIL with "method not found"

**Step 3: Write minimal implementation**

```rust
impl<'a> GraphTraverser<'a> {
    /// Get callee names for embedding (truncated priority order per G3)
    pub fn get_callees_for_embedding(
        &mut self,
        node_id: &str,
        limit: usize,
    ) -> Result<Vec<String>, GraphError> {
        let callees = self.get_callees(node_id)?;

        // Priority: decorated > frequency > alphabetical (G3)
        let mut sorted: Vec<_> = callees.into_iter().collect();
        sorted.sort_by(|a, b| {
            // Decorated nodes first
            let a_decorated = !a.decorators.is_empty();
            let b_decorated = !b.decorators.is_empty();
            match (a_decorated, b_decorated) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a.name.cmp(&b.name), // Alphabetical fallback
            }
        });

        Ok(sorted.into_iter().take(limit).map(|n| n.name).collect())
    }

    /// Get caller names for embedding
    pub fn get_callers_for_embedding(
        &mut self,
        node_id: &str,
        limit: usize,
    ) -> Result<Vec<String>, GraphError> {
        let callers = self.get_callers(node_id)?;

        let mut sorted: Vec<_> = callers.into_iter().collect();
        sorted.sort_by(|a, b| {
            let a_decorated = !a.decorators.is_empty();
            let b_decorated = !b.decorators.is_empty();
            match (a_decorated, b_decorated) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a.name.cmp(&b.name),
            }
        });

        Ok(sorted.into_iter().take(limit).map(|n| n.name).collect())
    }
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test --message-format=json -p codegraph-graph test_get_callees_for_embedding -- --nocapture`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/codegraph-graph/src/traverser.rs
git commit -m "feat(graph): add callee/caller queries for embedding with priority sorting"
```

---

### Task 14: Add Sibling Query for Embedding

**Files:**
- Modify: `crates/codegraph-graph/src/traverser.rs`
- Test: inline tests

**Step 1: Write the failing test**

```rust
#[test]
fn test_get_siblings_for_embedding() {
    let db = setup_test_graph();
    let mut queries = QueryBuilder::new(db.conn()).unwrap();
    let mut traverser = GraphTraverser::new(db.conn(), &mut queries);

    // method_a is in ClassA, method_b is also in ClassA
    let siblings = traverser.get_siblings_for_embedding("method_a", 8).unwrap();

    assert!(siblings.contains(&"method_b".to_string()));
    // Should not include self
    assert!(!siblings.contains(&"method_a".to_string()));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --message-format=json -p codegraph-graph test_get_siblings_for_embedding -- --nocapture`
Expected: FAIL

**Step 3: Write minimal implementation**

```rust
impl<'a> GraphTraverser<'a> {
    /// Get sibling names (other children of same parent container)
    pub fn get_siblings_for_embedding(
        &mut self,
        node_id: &str,
        limit: usize,
    ) -> Result<Vec<String>, GraphError> {
        // Find parent via "contains" edge
        let parents: Vec<Node> = self.queries.get_nodes_with_edge_to(
            self.conn,
            node_id,
            EdgeKind::Contains,
        )?;

        if parents.is_empty() {
            return Ok(Vec::new());
        }

        let parent = &parents[0];

        // Get all children of parent
        let children: Vec<Node> = self.queries.get_nodes_with_edge_from(
            self.conn,
            &parent.id.as_str(),
            EdgeKind::Contains,
        )?;

        // Filter out self, sort by decoration then alphabetically
        let mut siblings: Vec<_> = children
            .into_iter()
            .filter(|n| n.id.as_str() != node_id)
            .collect();

        siblings.sort_by(|a, b| {
            let a_decorated = !a.decorators.is_empty();
            let b_decorated = !b.decorators.is_empty();
            match (a_decorated, b_decorated) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a.name.cmp(&b.name),
            }
        });

        Ok(siblings.into_iter().take(limit).map(|n| n.name).collect())
    }
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test --message-format=json -p codegraph-graph test_get_siblings_for_embedding -- --nocapture`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/codegraph-graph/src/traverser.rs
git commit -m "feat(graph): add sibling query for embedding context"
```

---

### Task 15: Add Implements/Extends Query for Embedding

**Files:**
- Modify: `crates/codegraph-graph/src/traverser.rs`
- Test: inline tests

**Step 1: Write the failing test**

```rust
#[test]
fn test_get_inheritance_for_embedding() {
    let db = setup_test_graph();
    let mut queries = QueryBuilder::new(db.conn()).unwrap();
    let mut traverser = GraphTraverser::new(db.conn(), &mut queries);

    // ClassA implements InterfaceB, InterfaceC
    let inheritance = traverser.get_inheritance_for_embedding("class_a").unwrap();

    assert!(inheritance.implements.contains(&"InterfaceB".to_string()));
    assert!(inheritance.implements.contains(&"InterfaceC".to_string()));
}

#[test]
fn test_get_extends_for_embedding() {
    let db = setup_test_graph();
    let mut queries = QueryBuilder::new(db.conn()).unwrap();
    let mut traverser = GraphTraverser::new(db.conn(), &mut queries);

    // ChildClass extends ParentClass
    let inheritance = traverser.get_inheritance_for_embedding("child_class").unwrap();

    assert_eq!(inheritance.extends, Some("ParentClass".to_string()));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --message-format=json -p codegraph-graph test_get_inheritance -- --nocapture`
Expected: FAIL

**Step 3: Write minimal implementation**

```rust
/// Inheritance context for embedding
#[derive(Debug, Default)]
pub struct InheritanceContext {
    pub implements: Vec<String>,
    pub extends: Option<String>,
}

impl<'a> GraphTraverser<'a> {
    /// Get inheritance context (implements, extends) for a class/struct
    pub fn get_inheritance_for_embedding(
        &mut self,
        node_id: &str,
    ) -> Result<InheritanceContext, GraphError> {
        let mut context = InheritanceContext::default();

        // Get implements edges
        let implemented: Vec<Node> = self.queries.get_nodes_with_edge_from(
            self.conn,
            node_id,
            EdgeKind::Implements,
        )?;
        context.implements = implemented.into_iter().map(|n| n.name).collect();

        // Get extends edges
        let extended: Vec<Node> = self.queries.get_nodes_with_edge_from(
            self.conn,
            node_id,
            EdgeKind::Extends,
        )?;
        context.extends = extended.into_iter().next().map(|n| n.name);

        Ok(context)
    }
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test --message-format=json -p codegraph-graph test_get_inheritance -- --nocapture`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/codegraph-graph/src/traverser.rs
git commit -m "feat(graph): add implements/extends query for class inheritance context"
```

---

### Task 16: Implement Batched Graph Context Queries (B7)

**Files:**
- Create: `crates/codegraph-graph/src/batch_queries.rs`
- Modify: `crates/codegraph-graph/src/lib.rs`
- Test: inline tests

**Step 1: Write the failing test**

```rust
#[test]
fn test_batch_graph_context() {
    let db = setup_test_graph();
    let mut queries = QueryBuilder::new(db.conn()).unwrap();
    let batch_querier = BatchGraphQuerier::new(db.conn(), &mut queries);

    let node_ids = vec!["fn_a", "fn_b", "fn_c"];
    let config = EmbeddingTextConfig::default();

    let contexts = batch_querier.get_graph_contexts(&node_ids, &config).unwrap();

    assert_eq!(contexts.len(), 3);
    // Each context should have graph data
    for (node_id, context) in &contexts {
        assert!(node_ids.contains(&node_id.as_str()));
    }
}

#[test]
fn test_batch_query_performance() {
    let db = setup_large_test_graph(1000); // 1000 nodes
    let mut queries = QueryBuilder::new(db.conn()).unwrap();
    let batch_querier = BatchGraphQuerier::new(db.conn(), &mut queries);

    let node_ids: Vec<&str> = (0..1000).map(|i| format!("node_{}", i)).collect();
    let config = EmbeddingTextConfig::default();

    let start = std::time::Instant::now();
    let contexts = batch_querier.get_graph_contexts(&node_ids, &config).unwrap();
    let elapsed = start.elapsed();

    assert_eq!(contexts.len(), 1000);
    // Batch query should be fast (< 1 second for 1000 nodes)
    assert!(elapsed.as_secs() < 1, "Batch query took too long: {:?}", elapsed);
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --message-format=json -p codegraph-graph test_batch_graph_context -- --nocapture`
Expected: FAIL

**Step 3: Write minimal implementation**

```rust
// crates/codegraph-graph/src/batch_queries.rs
//! Batched graph queries for embedding generation (B7)

use crate::error::GraphError;
use codegraph_db::QueryBuilder;
use codegraph_types::EmbeddingConfig;
use codegraph_vectors::GraphContext;
use rusqlite::Connection;
use std::collections::HashMap;

/// Batch graph context querier
pub struct BatchGraphQuerier<'a> {
    conn: &'a Connection,
    queries: &'a mut QueryBuilder,
}

impl<'a> BatchGraphQuerier<'a> {
    pub fn new(conn: &'a Connection, queries: &'a mut QueryBuilder) -> Self {
        Self { conn, queries }
    }

    /// Get graph contexts for multiple nodes in a single batched query
    pub fn get_graph_contexts(
        &mut self,
        node_ids: &[&str],
        config: &EmbeddingConfig,
    ) -> Result<HashMap<String, GraphContext>, GraphError> {
        let mut contexts: HashMap<String, GraphContext> = HashMap::new();

        // Initialize empty contexts
        for &id in node_ids {
            contexts.insert(id.to_string(), GraphContext::default());
        }

        // Batch query for callees using CTE
        self.batch_query_callees(node_ids, config.max_callees, &mut contexts)?;

        // Batch query for callers
        self.batch_query_callers(node_ids, config.max_callers, &mut contexts)?;

        // Batch query for siblings
        self.batch_query_siblings(node_ids, config.max_siblings, &mut contexts)?;

        // Batch query for inheritance
        self.batch_query_inheritance(node_ids, &mut contexts)?;

        Ok(contexts)
    }

    fn batch_query_callees(
        &self,
        node_ids: &[&str],
        limit: usize,
        contexts: &mut HashMap<String, GraphContext>,
    ) -> Result<(), GraphError> {
        if node_ids.is_empty() {
            return Ok(());
        }

        // Build CTE query for all callees at once
        let placeholders: String = node_ids.iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 1))
            .collect::<Vec<_>>()
            .join(",");

        let sql = format!(r#"
            WITH callees AS (
                SELECT e.source AS caller_id, n.name AS callee_name,
                       CASE WHEN n.decorators != '[]' THEN 1 ELSE 0 END AS is_decorated,
                       ROW_NUMBER() OVER (
                           PARTITION BY e.source
                           ORDER BY CASE WHEN n.decorators != '[]' THEN 0 ELSE 1 END, n.name
                       ) AS rn
                FROM edges e
                JOIN nodes n ON e.target = n.id
                WHERE e.kind = 'calls' AND e.source IN ({})
            )
            SELECT caller_id, callee_name FROM callees WHERE rn <= ?
        "#, placeholders);

        let mut stmt = self.conn.prepare(&sql)?;

        let mut params: Vec<Box<dyn rusqlite::ToSql>> = node_ids
            .iter()
            .map(|&id| Box::new(id.to_string()) as Box<dyn rusqlite::ToSql>)
            .collect();
        params.push(Box::new(limit as i64));

        let params_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();

        let rows = stmt.query_map(params_refs.as_slice(), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;

        for row in rows {
            let (caller_id, callee_name) = row?;
            if let Some(ctx) = contexts.get_mut(&caller_id) {
                ctx.callees.push(callee_name);
            }
        }

        Ok(())
    }

    // Similar implementations for batch_query_callers, batch_query_siblings, batch_query_inheritance
    // ...
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test --message-format=json -p codegraph-graph test_batch_graph_context -- --nocapture`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/codegraph-graph/src/batch_queries.rs crates/codegraph-graph/src/lib.rs
git commit -m "feat(graph): implement batched graph context queries using CTEs (B7)"
```

---

### Task 17: Add Call Frequency Counting (G6)

**Files:**
- Modify: `crates/codegraph-graph/src/batch_queries.rs`
- Test: inline tests

**Step 1: Write the failing test**

```rust
#[test]
fn test_call_frequency_in_priority() {
    let db = setup_test_graph();
    // fn_popular is called by 10 functions, fn_rare is called by 1
    let mut queries = QueryBuilder::new(db.conn()).unwrap();
    let batch_querier = BatchGraphQuerier::new(db.conn(), &mut queries);

    let config = EmbeddingConfig {
        max_callees: 5,
        ..Default::default()
    };

    let contexts = batch_querier.get_graph_contexts(&["fn_caller"], &config).unwrap();
    let ctx = contexts.get("fn_caller").unwrap();

    // fn_popular should appear before fn_rare due to call frequency
    let popular_idx = ctx.callees.iter().position(|n| n == "fn_popular");
    let rare_idx = ctx.callees.iter().position(|n| n == "fn_rare");

    if let (Some(p), Some(r)) = (popular_idx, rare_idx) {
        assert!(p < r, "Popular function should come before rare function");
    }
}
```

**Step 2-5:** Similar TDD flow as previous tasks.

**Commit:**
```bash
git commit -m "feat(graph): add call frequency to truncation priority ordering (G6)"
```

---

### Task 18: Integrate Graph Context with Text Builder

**Files:**
- Modify: `crates/codegraph-vectors/src/text_builder.rs`
- Modify: `crates/codegraph-core/src/embedding.rs` (create if needed)
- Test: integration test

**Step 1: Write the failing test**

```rust
#[test]
fn test_full_embedding_pipeline() {
    let db = setup_indexed_test_project();
    let config = CodeGraphConfig::default();
    let embedding_service = EmbeddingService::new(&db, &config);

    let node_id = "test_function_id";
    let embedding = embedding_service.embed_node(node_id).unwrap();

    assert_eq!(embedding.len(), 768);
}
```

**Step 2-5:** Similar TDD flow.

**Commit:**
```bash
git commit -m "feat(core): integrate graph context with embedding text builder"
```

---

## Milestone 4: LSP Foundation (Tasks 19-26)

### Task 19: Define LspEnricher Trait

**Files:**
- Create: `crates/codegraph-lsp/src/lib.rs`
- Create: `crates/codegraph-lsp/src/enricher.rs`
- Create: `crates/codegraph-lsp/Cargo.toml`
- Test: inline tests

**Step 1: Write the failing test**

```rust
#[test]
fn test_lsp_enricher_trait() {
    // Trait should be object-safe
    let enrichers: Vec<Box<dyn LspEnricher>> = vec![];
    assert!(enrichers.is_empty());
}
```

**Step 2-3:** Define trait:

```rust
// crates/codegraph-lsp/src/enricher.rs
use async_trait::async_trait;

/// Result of an LSP hover query
#[derive(Debug, Clone)]
pub struct HoverResult {
    pub inferred_type: Option<String>,
    pub documentation: Option<String>,
}

/// Result of an LSP definition query
#[derive(Debug, Clone)]
pub struct DefinitionResult {
    pub file_path: String,
    pub line: u32,
    pub column: u32,
}

/// LSP enricher trait for language-specific implementations
#[async_trait]
pub trait LspEnricher: Send + Sync {
    /// Start the LSP server
    async fn start(&mut self, workspace_root: &Path) -> Result<(), LspError>;

    /// Query hover information at a position
    async fn hover(&self, file: &Path, line: u32, column: u32) -> Result<Option<HoverResult>, LspError>;

    /// Query definition at a position
    async fn definition(&self, file: &Path, line: u32, column: u32) -> Result<Option<DefinitionResult>, LspError>;

    /// Shutdown the LSP server
    async fn shutdown(&mut self) -> Result<(), LspError>;

    /// Check if server is ready (workspace initialized)
    fn is_ready(&self) -> bool;

    /// Get supported language
    fn language(&self) -> Language;
}
```

**Step 4-5:** Similar TDD flow.

**Commit:**
```bash
git commit -m "feat(lsp): define LspEnricher trait for language-specific implementations"
```

---

### Task 20: Add tower-lsp Client Infrastructure

**Files:**
- Modify: `crates/codegraph-lsp/Cargo.toml`
- Create: `crates/codegraph-lsp/src/client.rs`
- Test: inline tests

**Step 1:** Add dependencies:
```toml
[dependencies]
tower-lsp = "0.20"
tokio = { version = "1", features = ["full"] }
async-trait = "0.1"
```

**Step 2-5:** Implement base LSP client.

**Commit:**
```bash
git commit -m "feat(lsp): add tower-lsp client infrastructure"
```

---

### Task 21: Implement TypeScript Enricher

**Files:**
- Create: `crates/codegraph-lsp/src/typescript.rs`
- Test: integration test (requires typescript-language-server)

**Step 1: Write the failing test**

```rust
#[tokio::test]
#[ignore] // Requires typescript-language-server installed
async fn test_typescript_enricher_hover() {
    let mut enricher = TypeScriptEnricher::new();
    enricher.start(Path::new("/tmp/test-project")).await.unwrap();

    // Wait for workspace init (I10)
    tokio::time::sleep(Duration::from_secs(5)).await;

    let hover = enricher.hover(
        Path::new("/tmp/test-project/src/index.ts"),
        10, 5
    ).await.unwrap();

    assert!(hover.is_some());
}
```

**Step 2-5:** Implement TypeScript enricher with:
- Spawn `typescript-language-server --stdio`
- Handle workspace initialization (I10: wait up to 60s)
- Convert positions to UTF-16 (I9)
- Query timeout (I8: 5 seconds)

**Commit:**
```bash
git commit -m "feat(lsp): implement TypeScript LSP enricher"
```

---

### Task 22: Add Position Encoding Conversion (I9)

**Files:**
- Create: `crates/codegraph-lsp/src/encoding.rs`
- Test: inline tests

**Step 1: Write the failing test**

```rust
#[test]
fn test_byte_to_utf16_conversion() {
    // "Hello 🌍" - emoji is 4 bytes but 2 UTF-16 code units
    let text = "Hello 🌍 World";
    let byte_offset = 11; // Start of "World"

    let utf16_offset = byte_to_utf16(text, byte_offset);

    // "Hello " = 6, "🌍" = 2 (surrogate pair), " " = 1 = 9
    assert_eq!(utf16_offset, 9);
}

#[test]
fn test_utf16_to_byte_conversion() {
    let text = "Hello 🌍 World";
    let utf16_offset = 9;

    let byte_offset = utf16_to_byte(text, utf16_offset);

    assert_eq!(byte_offset, 11);
}
```

**Step 2-5:** Implement conversions.

**Commit:**
```bash
git commit -m "feat(lsp): add UTF-16 position encoding conversion (I9)"
```

---

### Task 23: Add LSP Server Lifecycle Management (L5)

**Files:**
- Create: `crates/codegraph-lsp/src/lifecycle.rs`
- Test: inline tests

**Step 1: Write the failing test**

```rust
#[tokio::test]
async fn test_server_lazy_spawn() {
    let mut manager = LspServerManager::new();

    // Server not started yet
    assert!(!manager.is_running(Language::TypeScript));

    // First query should spawn server
    manager.ensure_running(Language::TypeScript).await.unwrap();

    assert!(manager.is_running(Language::TypeScript));
}

#[tokio::test]
async fn test_server_kept_alive_during_indexing() {
    let mut manager = LspServerManager::new();
    manager.ensure_running(Language::TypeScript).await.unwrap();

    // Simulate indexing multiple files
    for _ in 0..10 {
        manager.query(Language::TypeScript, /* ... */).await.unwrap();
    }

    // Server should still be running
    assert!(manager.is_running(Language::TypeScript));
}
```

**Step 2-5:** Implement lifecycle management.

**Commit:**
```bash
git commit -m "feat(lsp): add lazy spawn and keep-alive server lifecycle (L5)"
```

---

### Task 24: Add LSP Error Handling (I6, I7)

**Files:**
- Create: `crates/codegraph-lsp/src/error.rs`
- Modify: `crates/codegraph-lsp/src/lifecycle.rs`
- Test: inline tests

**Step 1: Write the failing test**

```rust
#[tokio::test]
async fn test_retry_on_transient_failure() {
    let mut manager = LspServerManager::new();

    // Simulate server that fails twice then succeeds
    let result = manager.ensure_running_with_retry(Language::TypeScript, 3).await;

    assert!(result.is_ok());
}

#[tokio::test]
async fn test_skip_file_on_error() {
    let mut enricher = TypeScriptEnricher::new();
    enricher.start(Path::new("/tmp/test")).await.unwrap();

    // Query a file with syntax error
    let result = enricher.hover(
        Path::new("/tmp/test/broken.ts"),
        1, 1
    ).await;

    // Should return None, not error
    assert!(result.is_ok());
    assert!(result.unwrap().is_none());
}
```

**Step 2-5:** Implement error handling with retry and graceful degradation.

**Commit:**
```bash
git commit -m "feat(lsp): add retry logic and per-file error handling (I6, I7)"
```

---

### Task 25: Add Workspace Initialization Waiting (I10)

**Files:**
- Modify: `crates/codegraph-lsp/src/typescript.rs`
- Test: integration test

**Step 1: Write the failing test**

```rust
#[tokio::test]
#[ignore]
async fn test_wait_for_workspace_init() {
    let mut enricher = TypeScriptEnricher::new();
    let config = LspConfig {
        workspace_init_timeout_secs: 60,
        ..Default::default()
    };

    enricher.start_with_config(Path::new("/tmp/large-project"), &config).await.unwrap();

    // Should wait for project analysis before returning
    assert!(enricher.is_ready());

    // Queries should work immediately after start returns
    let hover = enricher.hover(Path::new("/tmp/large-project/src/index.ts"), 1, 1).await;
    assert!(hover.is_ok());
}
```

**Step 2-5:** Implement workspace init waiting.

**Commit:**
```bash
git commit -m "feat(lsp): add workspace initialization waiting with timeout (I10)"
```

---

### Task 26: Integrate LSP Enricher with Indexing Pipeline

**Files:**
- Modify: `crates/codegraph-core/src/indexer.rs`
- Test: integration test

**Step 1: Write the failing test**

```rust
#[tokio::test]
async fn test_indexing_with_lsp_enrichment() {
    let config = CodeGraphConfig {
        lsp: LspConfig {
            enabled: true,
            typescript: Some(LspServerConfig {
                enabled: true,
                server: "typescript-language-server".to_string(),
                args: vec!["--stdio".to_string()],
            }),
            ..Default::default()
        },
        ..Default::default()
    };

    let indexer = Indexer::new(&config);
    indexer.index(Path::new("/tmp/test-project")).await.unwrap();

    // Check that nodes have LSP-enriched fields
    let db = DatabaseConnection::open(config.db_path).unwrap();
    let node = get_node_by_name(&db, "someFunction").unwrap();

    assert!(node.inferred_type.is_some());
}
```

**Step 2-5:** Integrate LSP enrichment into indexing.

**Commit:**
```bash
git commit -m "feat(core): integrate LSP enrichment into indexing pipeline"
```

---

## Milestone 5: LSP Languages (Tasks 27-30)

### Task 27: Implement Dart Enricher

**Files:**
- Create: `crates/codegraph-lsp/src/dart.rs`
- Test: integration test

**Commit:**
```bash
git commit -m "feat(lsp): implement Dart LSP enricher"
```

---

### Task 28: Implement Rust Enricher

**Files:**
- Create: `crates/codegraph-lsp/src/rust_analyzer.rs`
- Test: integration test

**Commit:**
```bash
git commit -m "feat(lsp): implement Rust (rust-analyzer) LSP enricher"
```

---

### Task 29: Add Multi-Instance LSP Support (I5, I5a)

**Files:**
- Modify: `crates/codegraph-lsp/src/lifecycle.rs`
- Create: `crates/codegraph-lsp/src/pool.rs`
- Test: inline tests

**Commit:**
```bash
git commit -m "feat(lsp): add multi-instance LSP pool with work-stealing (I5, I5a)"
```

---

### Task 30: Add LSP Configuration Validation

**Files:**
- Modify: `crates/codegraph-lsp/src/lib.rs`
- Test: inline tests

**Commit:**
```bash
git commit -m "feat(lsp): add configuration validation and helpful error messages"
```

---

## Milestone 6: Incremental Updates (Tasks 31-38)

### Task 31: Create enrichment_deps Table Operations

**Files:**
- Modify: `crates/codegraph-db/src/queries.rs`
- Test: inline tests

**Commit:**
```bash
git commit -m "feat(db): add enrichment_deps table operations for dependency tracking"
```

---

### Task 32: Populate enrichment_deps from LSP Responses

**Files:**
- Modify: `crates/codegraph-lsp/src/enricher.rs`
- Test: integration test

**Commit:**
```bash
git commit -m "feat(lsp): populate enrichment_deps from definition responses"
```

---

### Task 33: Implement Selective Scope Query (L4a)

**Files:**
- Create: `crates/codegraph-sync/src/scope.rs`
- Test: inline tests

**Commit:**
```bash
git commit -m "feat(sync): implement selective scope for incremental LSP enrichment (L4a)"
```

---

### Task 34: Add Edge Change Detection (I11)

**Files:**
- Modify: `crates/codegraph-sync/src/incremental.rs`
- Test: inline tests

**Commit:**
```bash
git commit -m "feat(sync): add edge change detection for re-embedding (I11)"
```

---

### Task 35: Add File Locking (I12)

**Files:**
- Create: `crates/codegraph-sync/src/lock.rs`
- Test: inline tests

**Commit:**
```bash
git commit -m "feat(sync): add file locking during indexing (I12)"
```

---

### Task 36: Implement Full Re-embed Trigger Detection (I13)

**Files:**
- Modify: `crates/codegraph-core/src/embedding.rs`
- Test: inline tests

**Commit:**
```bash
git commit -m "feat(core): detect full re-embed triggers (schema, config, force flag) (I13)"
```

---

### Task 37: Add Cascade Depth Configuration (I3)

**Files:**
- Modify: `crates/codegraph-sync/src/scope.rs`
- Test: inline tests

**Commit:**
```bash
git commit -m "feat(sync): add configurable cascade depth for dependency tracking (I3)"
```

---

### Task 38: Integrate Incremental Updates with Sync Command

**Files:**
- Modify: `crates/codegraph-cli/src/commands.rs`
- Test: integration test

**Commit:**
```bash
git commit -m "feat(cli): integrate incremental enrichment updates with sync command"
```

---

## Milestone 7: New Extraction (Tasks 39-44)

### Task 39: Extract Code Snippets During Parsing

**Files:**
- Modify: `crates/codegraph-extraction/src/tree_sitter_extractor.rs`
- Test: inline tests

**Commit:**
```bash
git commit -m "feat(extraction): extract code snippets up to 50 lines (E4)"
```

---

### Task 40: Extract Thrown Errors from AST

**Files:**
- Modify: `crates/codegraph-extraction/src/tree_sitter_extractor.rs`
- Test: inline tests (TypeScript, Python, Rust)

**Commit:**
```bash
git commit -m "feat(extraction): extract thrown errors from throw statements and annotations (E8)"
```

---

### Task 41: Implement Test Convention Detection (E6a)

**Files:**
- Create: `crates/codegraph-extraction/src/test_detection.rs`
- Test: inline tests per language

**Commit:**
```bash
git commit -m "feat(extraction): implement language-specific test convention detection (E6a)"
```

---

### Task 42: Add Test Association via Import Analysis

**Files:**
- Modify: `crates/codegraph-resolution/src/resolver.rs`
- Test: integration test

**Commit:**
```bash
git commit -m "feat(resolution): add test association via import analysis fallback (E6)"
```

---

### Task 43: Extract Package Name from Manifests (M1)

**Files:**
- Create: `crates/codegraph-extraction/src/package_detection.rs`
- Test: inline tests (package.json, Cargo.toml, pubspec.yaml)

**Commit:**
```bash
git commit -m "feat(extraction): extract package names from manifests (M1)"
```

---

### Task 44: Add Workspace Detection for Mono-repos (M3)

**Files:**
- Modify: `crates/codegraph-extraction/src/package_detection.rs`
- Test: inline tests

**Commit:**
```bash
git commit -m "feat(extraction): add workspace detection for mono-repo package names (M3)"
```

---

## Milestone 8: Quality & Polish (Tasks 45-50)

### Task 45: Create Evaluation Test Cases for Enrichment

**Files:**
- Create: `crates/codegraph-core/src/evaluation.rs`
- Create: `tests/evaluation/enrichment_cases.json`
- Test: inline tests

**Commit:**
```bash
git commit -m "feat(eval): create enrichment-specific evaluation test cases"
```

---

### Task 46: Implement A/B Comparison Mode (Q1)

**Files:**
- Modify: `crates/codegraph-core/src/evaluation.rs`
- Test: inline tests

**Commit:**
```bash
git commit -m "feat(eval): implement A/B comparison for baseline vs enriched embeddings (Q1)"
```

---

### Task 47: Add CLI Flags for Enrichment Control

**Files:**
- Modify: `crates/codegraph-cli/src/commands.rs`
- Modify: `crates/codegraph-cli/src/main.rs`
- Test: CLI integration test

**Commit:**
```bash
git commit -m "feat(cli): add --force-reembed, --no-lsp, --lsp-scope flags"
```

---

### Task 48: Add Git Activity Boost Option (B8)

**Files:**
- Create: `crates/codegraph-vectors/src/git_boost.rs`
- Test: inline tests

**Commit:**
```bash
git commit -m "feat(vectors): add optional git activity boost for truncation priority (B8)"
```

---

### Task 49: Add Embedding Version Tracking

**Files:**
- Modify: `crates/codegraph-core/src/embedding.rs`
- Test: inline tests

**Commit:**
```bash
git commit -m "feat(core): add embedding version tracking in metadata table"
```

---

### Task 50: Final Integration and Documentation

**Files:**
- Update: `CLAUDE.md`
- Update: `README.md` (if exists)
- Test: full integration test

**Commit:**
```bash
git commit -m "docs: update documentation for embedding enrichment feature"
```

---

## Summary

This implementation plan covers the complete Embedding Enrichment feature in **50 tasks** across **8 milestones**:

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

## Decision Traceability

Each task maps to design decisions:

| Task | Decisions |
|------|-----------|
| 1 | Schema changes for L1-L5, E4-E8, M1-M3 |
| 2-3 | C1, L4-L5, B1-B3 |
| 4 | I13 metadata tracking |
| 5-7 | E1-E3, B1-B6 |
| 8-9 | L1-L3, M1-M3 |
| 10-11 | E7-E8, E4 |
| 12 | Integration |
| 13-15 | G1-G5 |
| 16-17 | B7, G6 |
| 18 | Integration |
| 19-26 | L1-L5, I6-I10 |
| 27-30 | L3, I5-I5a |
| 31-38 | I1-I4, I11-I13 |
| 39-44 | E4-E8, M1-M3 |
| 45-50 | Q1, B8, I13 |

---

Plan complete. Ready for adversarial review.
