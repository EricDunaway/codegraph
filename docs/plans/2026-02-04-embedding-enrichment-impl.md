# Embedding Enrichment Implementation Plan (v2 - Post-Review)

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Enrich CodeGraph's semantic search embeddings with LSP-derived types, graph context, code snippets, and module membership to improve search relevance.

**Architecture:** Separate enrichment phases after extraction (Extract → LSP → Embed with inline graph computation). LSP queries run during indexing with configurable scope. Graph context computed at embedding time from edges table. Token budget enforced with tiered truncation.

**Tech Stack:** Rust, SQLite, ONNX embeddings, tower-lsp client, tiktoken (cl100k_base proxy), serde_json

**Design Document:** `docs/plans/2026-02-04-embedding-enrichment-design.md`
**Review Document:** `docs/plans/2026-02-04-embedding-enrichment-review.md`

---

## Key Technical Decisions (From Review)

1. **Async/Sync Bridge:** Use `tokio::runtime::Runtime::block_on()` to call async LSP from sync indexing code
2. **Node Struct:** Add all enrichment fields as `Option<T>` or `Vec<T>` with serde defaults
3. **Test Helpers:** Define inline in each test module using `tempfile` crate

---

## Milestone Overview

| # | Milestone | Tasks | Focus |
|---|-----------|-------|-------|
| 1 | Schema & Types | 1-6 | Database migrations, Node struct, config types |
| 2 | Embedding Text | 7-14 | New embedding template with existing fields, token budgeting |
| 3 | Graph Context | 15-20 | Batched graph queries, truncation priority |
| 4 | LSP Foundation | 21-30 | Crate setup, LSP trait, TypeScript enricher |
| 5 | LSP Languages | 31-34 | Dart, Rust enrichers |
| 6 | Incremental Updates | 35-42 | Dependency tracking, selective scope |
| 7 | New Extraction | 43-48 | Code snippets, thrown errors, test association |
| 8 | Quality & Polish | 49-54 | Evaluation framework, CLI flags |

**Total: 54 tasks** (4 new tasks added from review)

---

## Milestone 1: Schema & Types (Tasks 1-6)

### Task 1: Add Schema Migration Infrastructure

**Files:**
- Create: `crates/codegraph-db/src/migrations.rs`
- Modify: `crates/codegraph-db/src/lib.rs`

**Step 1: Write the failing test**

```rust
// crates/codegraph-db/src/migrations.rs
#[cfg(test)]
mod tests {
    use super::*;
    use crate::DatabaseConnection;

    #[test]
    fn test_get_schema_version_initial() {
        let db = DatabaseConnection::open_in_memory().unwrap();
        // Schema is already v1 from initial creation
        let version = get_schema_version(db.conn()).unwrap();
        assert_eq!(version, 1);
    }

    #[test]
    fn test_migration_to_v2() {
        let db = DatabaseConnection::open_in_memory().unwrap();

        // Run migration
        migrate_to_v2(db.conn()).unwrap();

        // Check version updated
        let version = get_schema_version(db.conn()).unwrap();
        assert_eq!(version, 2);

        // Verify new columns exist by inserting with them
        db.conn().execute(
            "INSERT INTO nodes (id, kind, name, qualified_name, file_path, language,
             start_line, end_line, start_column, end_column, updated_at,
             inferred_type, resolved_import_path, code_snippet, thrown_errors,
             test_names, package_name)
             VALUES ('test', 'function', 'test', 'test', 'test.rs', 'rust',
                     1, 1, 0, 0, 0, 'String', NULL, NULL, '[]', '[]', NULL)",
            [],
        ).unwrap();
    }

    #[test]
    fn test_migration_idempotent() {
        let db = DatabaseConnection::open_in_memory().unwrap();

        // Run migration twice
        migrate_to_v2(db.conn()).unwrap();
        migrate_to_v2(db.conn()).unwrap();

        // Should still be v2
        let version = get_schema_version(db.conn()).unwrap();
        assert_eq!(version, 2);
    }
}
```

**Step 2:** Run test to verify it fails
```bash
cargo test --message-format=json -p codegraph-db test_get_schema_version -- --nocapture
```

**Step 3: Write implementation**

```rust
// crates/codegraph-db/src/migrations.rs
//! Database schema migrations

use rusqlite::Connection;
use crate::error::DbError;

/// Get current schema version
pub fn get_schema_version(conn: &Connection) -> Result<u32, DbError> {
    let version: u32 = conn.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_version",
        [],
        |row| row.get(0),
    )?;
    Ok(version)
}

/// Migrate schema from v1 to v2 (add enrichment columns)
pub fn migrate_to_v2(conn: &Connection) -> Result<(), DbError> {
    let current = get_schema_version(conn)?;
    if current >= 2 {
        return Ok(()); // Already migrated
    }

    // Use transaction for atomicity
    conn.execute_batch(r#"
        BEGIN TRANSACTION;

        -- LSP enrichment fields
        ALTER TABLE nodes ADD COLUMN inferred_type TEXT;
        ALTER TABLE nodes ADD COLUMN resolved_import_path TEXT;

        -- New extraction fields
        ALTER TABLE nodes ADD COLUMN code_snippet TEXT;
        ALTER TABLE nodes ADD COLUMN thrown_errors TEXT DEFAULT '[]';
        ALTER TABLE nodes ADD COLUMN test_names TEXT DEFAULT '[]';
        ALTER TABLE nodes ADD COLUMN package_name TEXT;

        -- Dependency tracking for incremental LSP updates
        CREATE TABLE IF NOT EXISTS enrichment_deps (
            node_id TEXT NOT NULL,
            depends_on_file TEXT NOT NULL,
            PRIMARY KEY (node_id, depends_on_file),
            FOREIGN KEY (node_id) REFERENCES nodes(id) ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS idx_enrichment_deps_file ON enrichment_deps(depends_on_file);

        -- Metadata for version tracking
        CREATE TABLE IF NOT EXISTS metadata (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );

        -- Record migration
        INSERT INTO schema_version (version, applied_at, description)
        VALUES (2, strftime('%s', 'now'), 'Add enrichment columns');

        COMMIT;
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

**Step 4:** Run test to verify it passes
**Step 5:** Commit
```bash
git add crates/codegraph-db/src/migrations.rs crates/codegraph-db/src/lib.rs
git commit -m "feat(db): add schema migration infrastructure with v2 enrichment columns"
```

---

### Task 2: Update Node Struct with Enrichment Fields

**Files:**
- Modify: `crates/codegraph-types/src/lib.rs`

**Step 1: Write the failing test**

```rust
#[test]
fn test_node_enrichment_fields() {
    let mut node = Node::new(
        "test-id",
        NodeKind::Function,
        "test",
        "test::test",
        "test.rs",
        Language::Rust,
        1, 10,
    );

    // New enrichment fields should exist and default to None/empty
    assert!(node.inferred_type.is_none());
    assert!(node.resolved_import_path.is_none());
    assert!(node.code_snippet.is_none());
    assert!(node.thrown_errors.is_empty());
    assert!(node.test_names.is_empty());
    assert!(node.package_name.is_none());

    // Should be settable
    node.inferred_type = Some("Promise<void>".to_string());
    node.thrown_errors = vec!["AuthError".to_string()];

    assert_eq!(node.inferred_type.as_deref(), Some("Promise<void>"));
    assert_eq!(node.thrown_errors.len(), 1);
}
```

**Step 2:** Run test to verify it fails

**Step 3: Write implementation**

Add to `Node` struct in `crates/codegraph-types/src/lib.rs`:

```rust
pub struct Node {
    // ... existing fields ...

    /// Inferred type from LSP hover (LSP enrichment)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inferred_type: Option<String>,

    /// Resolved import path from LSP definition (LSP enrichment)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_import_path: Option<String>,

    /// Code snippet (first N lines of body)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_snippet: Option<String>,

    /// Thrown error types (JSON array in DB)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub thrown_errors: Vec<String>,

    /// Associated test names (JSON array in DB)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub test_names: Vec<String>,

    /// Package name (from manifest or directory path)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_name: Option<String>,
}
```

Update `Node::new()` to initialize new fields:

```rust
impl Node {
    pub fn new(/* existing params */) -> Self {
        Self {
            // ... existing field initializations ...
            inferred_type: None,
            resolved_import_path: None,
            code_snippet: None,
            thrown_errors: Vec::new(),
            test_names: Vec::new(),
            package_name: None,
        }
    }
}
```

**Step 4:** Run test to verify it passes
**Step 5:** Commit
```bash
git add crates/codegraph-types/src/lib.rs
git commit -m "feat(types): add enrichment fields to Node struct"
```

---

### Task 3: Update QueryBuilder for Enrichment Columns

**Files:**
- Modify: `crates/codegraph-db/src/queries.rs`

**Step 1: Write the failing test**

```rust
#[test]
fn test_insert_and_read_enriched_node() {
    let db = DatabaseConnection::open_in_memory().unwrap();
    migrations::run_migrations(db.conn()).unwrap();
    let queries = QueryBuilder::new(db.conn()).unwrap();

    let mut node = Node::new(
        "enriched-node",
        NodeKind::Function,
        "processPayment",
        "PaymentService.processPayment",
        "src/payment.ts",
        Language::TypeScript,
        10, 25,
    );
    node.inferred_type = Some("Promise<Receipt>".to_string());
    node.thrown_errors = vec!["PaymentError".to_string(), "ValidationError".to_string()];
    node.package_name = Some("@myapp/payments".to_string());

    // Insert
    queries.insert_node(db.conn(), &node).unwrap();

    // Read back
    let retrieved = queries.get_node_by_id(db.conn(), "enriched-node").unwrap().unwrap();

    assert_eq!(retrieved.inferred_type.as_deref(), Some("Promise<Receipt>"));
    assert_eq!(retrieved.thrown_errors.len(), 2);
    assert_eq!(retrieved.package_name.as_deref(), Some("@myapp/payments"));
}
```

**Step 2:** Run test to verify it fails

**Step 3: Write implementation**

Update `insert_node` SQL in `queries.rs`:

```rust
pub fn insert_node(&self, conn: &Connection, node: &Node) -> Result<(), DbError> {
    conn.execute(
        r#"
        INSERT INTO nodes (
            id, kind, name, qualified_name, file_path, language,
            start_line, end_line, start_column, end_column,
            docstring, signature, visibility,
            is_exported, is_async, is_static, is_abstract,
            decorators, type_parameters, updated_at,
            inferred_type, resolved_import_path, code_snippet,
            thrown_errors, test_names, package_name
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
            ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20,
            ?21, ?22, ?23, ?24, ?25, ?26
        )
        ON CONFLICT(id) DO UPDATE SET
            kind = ?2, name = ?3, qualified_name = ?4, file_path = ?5,
            language = ?6, start_line = ?7, end_line = ?8,
            start_column = ?9, end_column = ?10, docstring = ?11,
            signature = ?12, visibility = ?13, is_exported = ?14,
            is_async = ?15, is_static = ?16, is_abstract = ?17,
            decorators = ?18, type_parameters = ?19, updated_at = ?20,
            inferred_type = ?21, resolved_import_path = ?22, code_snippet = ?23,
            thrown_errors = ?24, test_names = ?25, package_name = ?26
        "#,
        params![
            node.id.as_str(),
            node.kind.as_str(),
            node.name,
            node.qualified_name,
            node.file_path,
            node.language.as_str(),
            node.start_line,
            node.end_line,
            node.start_column,
            node.end_column,
            node.docstring,
            node.signature,
            node.visibility.map(|v| v.as_str()),
            node.is_exported,
            node.is_async,
            node.is_static,
            node.is_abstract,
            serde_json::to_string(&node.decorators).unwrap(),
            serde_json::to_string(&node.type_parameters).unwrap(),
            node.updated_at,
            node.inferred_type,
            node.resolved_import_path,
            node.code_snippet,
            serde_json::to_string(&node.thrown_errors).unwrap(),
            serde_json::to_string(&node.test_names).unwrap(),
            node.package_name,
        ],
    )?;
    Ok(())
}
```

Update `get_node_by_id` to read new columns (add to the row mapping):

```rust
// In the row mapping closure
inferred_type: row.get("inferred_type")?,
resolved_import_path: row.get("resolved_import_path")?,
code_snippet: row.get("code_snippet")?,
thrown_errors: row.get::<_, Option<String>>("thrown_errors")?
    .map(|s| serde_json::from_str(&s).unwrap_or_default())
    .unwrap_or_default(),
test_names: row.get::<_, Option<String>>("test_names")?
    .map(|s| serde_json::from_str(&s).unwrap_or_default())
    .unwrap_or_default(),
package_name: row.get("package_name")?,
```

**Step 4:** Run test to verify it passes
**Step 5:** Commit
```bash
git add crates/codegraph-db/src/queries.rs
git commit -m "feat(db): update QueryBuilder to handle enrichment columns"
```

---

### Task 4: Add Enrichment Config Types

**Files:**
- Modify: `crates/codegraph-types/src/lib.rs`

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
    let config = EmbeddingTextConfig::default();
    assert_eq!(config.max_tokens, 2000);
    assert_eq!(config.max_callees, 10);
    assert_eq!(config.max_callers, 5);
    assert_eq!(config.max_siblings, 8);
    assert_eq!(config.max_snippet_lines, 50);
    assert!(!config.git_activity_boost);
}

#[test]
fn test_lsp_config_serialization() {
    let config = LspConfig {
        enabled: true,
        typescript: Some(LspServerConfig {
            enabled: true,
            server: "typescript-language-server".to_string(),
            args: vec!["--stdio".to_string()],
        }),
        ..Default::default()
    };

    let json = serde_json::to_string(&config).unwrap();
    assert!(json.contains("typescript-language-server"));

    let parsed: LspConfig = serde_json::from_str(&json).unwrap();
    assert!(parsed.enabled);
}
```

**Step 2:** Run test to verify it fails

**Step 3: Write implementation**

```rust
// Add to crates/codegraph-types/src/lib.rs

/// LSP enrichment scope (L4)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LspScope {
    /// Comprehensive on index, selective on sync (default)
    #[default]
    Hybrid,
    /// Query all nodes
    Comprehensive,
    /// Query only changed/missing nodes
    Selective,
}

/// Action when LSP is unavailable (I6)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LspUnavailableAction {
    /// Fail the operation (default)
    #[default]
    Fail,
    /// Continue without LSP enrichment
    Degrade,
}

/// LSP server configuration for a specific language
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspServerConfig {
    pub enabled: bool,
    pub server: String,
    #[serde(default)]
    pub args: Vec<String>,
}

/// LSP configuration (L1-L5)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LspConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub typescript: Option<LspServerConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dart: Option<LspServerConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rust: Option<LspServerConfig>,
}

/// Enrichment configuration (I1-I13)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrichmentConfig {
    #[serde(default)]
    pub lsp_scope: LspScope,
    #[serde(default = "default_cascade_depth")]
    pub cascade_depth: u32,
    #[serde(default = "default_one")]
    pub lsp_instances: u32,
    #[serde(default)]
    pub on_lsp_unavailable: LspUnavailableAction,
    #[serde(default = "default_query_timeout")]
    pub query_timeout_secs: u64,
    #[serde(default = "default_workspace_init_timeout")]
    pub workspace_init_timeout_secs: u64,
}

fn default_cascade_depth() -> u32 { 1 }
fn default_one() -> u32 { 1 }
fn default_query_timeout() -> u64 { 5 }
fn default_workspace_init_timeout() -> u64 { 60 }

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

/// Embedding text configuration (B1-B8, G2)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingTextConfig {
    #[serde(default = "default_max_tokens")]
    pub max_tokens: usize,
    #[serde(default = "default_max_callees")]
    pub max_callees: usize,
    #[serde(default = "default_max_callers")]
    pub max_callers: usize,
    #[serde(default = "default_max_siblings")]
    pub max_siblings: usize,
    #[serde(default = "default_max_snippet_lines")]
    pub max_snippet_lines: usize,
    #[serde(default)]
    pub git_activity_boost: bool,
}

fn default_max_tokens() -> usize { 2000 }
fn default_max_callees() -> usize { 10 }
fn default_max_callers() -> usize { 5 }
fn default_max_siblings() -> usize { 8 }
fn default_max_snippet_lines() -> usize { 50 }

impl Default for EmbeddingTextConfig {
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

**Step 4:** Run test to verify it passes
**Step 5:** Commit
```bash
git add crates/codegraph-types/src/lib.rs
git commit -m "feat(types): add enrichment and embedding config types"
```

---

### Task 5: Add JSON Config File Parsing

**Files:**
- Modify: `crates/codegraph-core/src/config.rs`

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

#[test]
fn test_load_config_defaults_when_missing() {
    use tempfile::TempDir;

    let temp = TempDir::new().unwrap();
    // No config file

    let config = CodeGraphConfig::load(temp.path()).unwrap();
    assert!(!config.lsp.enabled);
    assert_eq!(config.enrichment.lsp_scope, LspScope::Hybrid);
    assert_eq!(config.embedding.max_tokens, 2000);
}
```

**Step 2-5:** Similar TDD flow.

**Commit:**
```bash
git commit -m "feat(config): add JSON config file parsing with LSP/enrichment settings"
```

---

### Task 6: Add Metadata Table Operations

**Files:**
- Modify: `crates/codegraph-db/src/queries.rs`

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

    // Update existing
    queries.set_metadata(db.conn(), "embedding_version", "2").unwrap();
    let version = queries.get_metadata(db.conn(), "embedding_version").unwrap();
    assert_eq!(version, Some("2".to_string()));

    // Non-existent key
    let missing = queries.get_metadata(db.conn(), "nonexistent").unwrap();
    assert!(missing.is_none());

    // Delete
    queries.delete_metadata(db.conn(), "config_hash").unwrap();
    let deleted = queries.get_metadata(db.conn(), "config_hash").unwrap();
    assert!(deleted.is_none());
}
```

**Step 2-5:** Similar TDD flow.

**Commit:**
```bash
git commit -m "feat(db): add metadata table operations for version tracking"
```

---

## Milestone 2: Embedding Text (Tasks 7-14)

### Task 7: Create Embedding Text Builder Module

**Files:**
- Create: `crates/codegraph-vectors/src/text_builder.rs`
- Modify: `crates/codegraph-vectors/src/lib.rs`

**Step 1: Write the failing test**

```rust
// crates/codegraph-vectors/src/text_builder.rs
#[cfg(test)]
mod tests {
    use super::*;
    use codegraph_types::{Node, NodeKind, Language};

    fn make_test_node() -> Node {
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
        let node = make_test_node();
        let config = EmbeddingTextConfig::default();
        let builder = EmbeddingTextBuilder::new(config);

        let text = builder.build_text(&node, &GraphContext::default(), &NodeEnrichment::default());

        // Decorators first (E1)
        assert!(text.starts_with("@Controller"), "Should start with decorators: {}", text);
        // Contains kind and name
        assert!(text.contains("function processPayment"));
        // Contains file path
        assert!(text.contains("src/services/payment.ts"));
        // Contains signature
        assert!(text.contains("async processPayment(order: Order)"));
    }

    #[test]
    fn test_embedding_without_decorators() {
        let mut node = make_test_node();
        node.decorators = vec![];

        let config = EmbeddingTextConfig::default();
        let builder = EmbeddingTextBuilder::new(config);

        let text = builder.build_text(&node, &GraphContext::default(), &NodeEnrichment::default());

        // Should start with kind + name
        assert!(text.starts_with("function processPayment"));
    }
}
```

**Step 2-5:** Implement `EmbeddingTextBuilder` with:
- `GraphContext` struct (callees, callers, siblings, implements, extends)
- `NodeEnrichment` struct (lsp fields, package_name, thrown_errors, test_names, code_snippet)
- `build_text()` method following template from design doc

**Commit:**
```bash
git commit -m "feat(vectors): add embedding text builder with decorator-first ordering"
```

---

### Task 8: Add Token Counting with tiktoken

**Files:**
- Modify: `crates/codegraph-vectors/Cargo.toml`
- Modify: `crates/codegraph-vectors/src/text_builder.rs`

**Step 1: Write the failing test**

```rust
#[test]
fn test_token_counting() {
    let counter = TokenCounter::new().expect("Failed to create token counter");

    // Short text
    let count = counter.count_tokens("Hello world");
    assert!(count > 0 && count < 10, "Expected 2-3 tokens, got {}", count);

    // Code-like text
    let code = "function processPayment(order: Order): Promise<Receipt> { return this.gateway.charge(order.total); }";
    let count = counter.count_tokens(code);
    assert!(count > 15 && count < 40, "Expected ~25 tokens, got {}", count);
}
```

**Step 2:** Add dependency to Cargo.toml:
```toml
tiktoken-rs = "0.6"  # Verify latest version
```

**Step 3: Write implementation**

```rust
use tiktoken_rs::cl100k_base;

/// Token counter using tiktoken cl100k_base as proxy for embedding model (B5)
pub struct TokenCounter {
    bpe: tiktoken_rs::CoreBPE,
}

impl TokenCounter {
    pub fn new() -> Result<Self, VectorError> {
        let bpe = cl100k_base()
            .map_err(|e| VectorError::TokenizerFailed(e.to_string()))?;
        Ok(Self { bpe })
    }

    /// Count tokens in text
    pub fn count_tokens(&self, text: &str) -> usize {
        self.bpe.encode_with_special_tokens(text).len()
    }
}
```

**Step 4-5:** Run test, commit.

**Commit:**
```bash
git commit -m "feat(vectors): add token counting with tiktoken cl100k_base proxy (B5)"
```

---

### Task 9: Implement Tiered Truncation Algorithm

**Files:**
- Modify: `crates/codegraph-vectors/src/text_builder.rs`

**Step 1: Write the failing test**

```rust
#[test]
fn test_truncation_respects_budget() {
    let config = EmbeddingTextConfig {
        max_tokens: 50,  // Very small budget
        ..Default::default()
    };
    let builder = EmbeddingTextBuilder::new(config);
    let counter = TokenCounter::new().unwrap();

    let mut node = make_test_node();
    node.docstring = Some("A".repeat(1000)); // Very long docstring

    let text = builder.build_text_with_budget(
        &node,
        &GraphContext::default(),
        &NodeEnrichment::default(),
        &counter,
    );

    let tokens = counter.count_tokens(&text);
    assert!(tokens <= 50, "Should respect budget: {} tokens", tokens);
}

#[test]
fn test_tier1_overflow_truncates_decorators() {
    let config = EmbeddingTextConfig {
        max_tokens: 30,  // Very small
        ..Default::default()
    };
    let builder = EmbeddingTextBuilder::new(config);
    let counter = TokenCounter::new().unwrap();

    let mut node = make_test_node();
    // 50 decorators should trigger overflow protection (B6)
    node.decorators = (0..50).map(|i| format!("@Decorator{}", i)).collect();

    let text = builder.build_text_with_budget(
        &node,
        &GraphContext::default(),
        &NodeEnrichment::default(),
        &counter,
    );

    // Should have max 10 decorators after truncation
    let decorator_count = text.matches("@Decorator").count();
    assert!(decorator_count <= 10, "Should limit decorators to 10, found {}", decorator_count);
}

#[test]
fn test_tier1_overflow_truncates_signature() {
    let config = EmbeddingTextConfig {
        max_tokens: 30,
        ..Default::default()
    };
    let builder = EmbeddingTextBuilder::new(config);
    let counter = TokenCounter::new().unwrap();

    let mut node = make_test_node();
    node.decorators = vec![];
    // Very long signature (500+ chars)
    node.signature = Some(format!("function veryLongName({}): void", "param: Type, ".repeat(50)));

    let text = builder.build_text_with_budget(
        &node,
        &GraphContext::default(),
        &NodeEnrichment::default(),
        &counter,
    );

    // Signature should be truncated to ~200 chars
    let sig_start = text.find("function veryLongName").unwrap_or(0);
    let sig_line = text[sig_start..].lines().next().unwrap_or("");
    assert!(sig_line.len() <= 210, "Signature should be truncated, len={}", sig_line.len());
}
```

**Step 2-5:** Implement tiered truncation per design (B3, B6).

**Commit:**
```bash
git commit -m "feat(vectors): implement tiered truncation with overflow protection (B3, B6)"
```

---

### Task 10: Add Graph Context to Embedding Text

**Files:**
- Modify: `crates/codegraph-vectors/src/text_builder.rs`

**Step 1: Write the failing test**

```rust
#[test]
fn test_graph_context_in_embedding() {
    let config = EmbeddingTextConfig::default();
    let builder = EmbeddingTextBuilder::new(config);

    let node = make_test_node();
    let context = GraphContext {
        callees: vec!["validateOrder".to_string(), "chargeCard".to_string()],
        callers: vec!["handleCheckout".to_string()],
        siblings: vec!["refundPayment".to_string(), "getPaymentStatus".to_string()],
        implements: vec![],
        extends: None,
    };

    let text = builder.build_text(&node, &context, &NodeEnrichment::default());

    assert!(text.contains("calls: validateOrder, chargeCard"));
    assert!(text.contains("called by: handleCheckout"));
    assert!(text.contains("siblings: refundPayment, getPaymentStatus"));
}

#[test]
fn test_graph_context_respects_limits() {
    let config = EmbeddingTextConfig {
        max_callees: 2,
        max_callers: 1,
        max_siblings: 2,
        ..Default::default()
    };
    let builder = EmbeddingTextBuilder::new(config);

    let node = make_test_node();
    let context = GraphContext {
        callees: vec!["a".to_string(), "b".to_string(), "c".to_string(), "d".to_string()],
        callers: vec!["x".to_string(), "y".to_string(), "z".to_string()],
        siblings: vec!["s1".to_string(), "s2".to_string(), "s3".to_string()],
        ..Default::default()
    };

    let text = builder.build_text(&node, &context, &NodeEnrichment::default());

    // Should only have first 2 callees
    assert!(text.contains("calls: a, b"));
    assert!(!text.contains("c") || !text.contains("calls:"));
}

#[test]
fn test_class_inheritance_in_embedding() {
    let config = EmbeddingTextConfig::default();
    let builder = EmbeddingTextBuilder::new(config);

    let mut node = make_test_node();
    node.kind = NodeKind::Class;

    let context = GraphContext {
        implements: vec!["PaymentProcessor".to_string(), "Auditable".to_string()],
        extends: Some("BaseService".to_string()),
        ..Default::default()
    };

    let text = builder.build_text(&node, &context, &NodeEnrichment::default());

    assert!(text.contains("implements: PaymentProcessor, Auditable"));
    assert!(text.contains("extends: BaseService"));
}
```

**Step 2-5:** Implement graph context rendering.

**Commit:**
```bash
git commit -m "feat(vectors): add graph context (callees, callers, siblings, inheritance) to embedding"
```

---

### Task 11: Add Enrichment Fields to Embedding Text

**Files:**
- Modify: `crates/codegraph-vectors/src/text_builder.rs`

**Step 1: Write the failing test**

```rust
#[test]
fn test_lsp_enrichment_in_embedding() {
    let config = EmbeddingTextConfig::default();
    let builder = EmbeddingTextBuilder::new(config);

    let node = make_test_node();
    let enrichment = NodeEnrichment {
        inferred_type: Some("Promise<Receipt>".to_string()),
        resolved_import_path: None,
        package_name: Some("@myapp/payments".to_string()),
        thrown_errors: vec!["PaymentError".to_string()],
        test_names: vec!["should process valid payment".to_string()],
        code_snippet: None,
    };

    let text = builder.build_text(&node, &GraphContext::default(), &enrichment);

    assert!(text.contains("type: Promise<Receipt>"));
    assert!(text.contains("package: @myapp/payments"));
    assert!(text.contains("throws: PaymentError"));
    assert!(text.contains("tested by: should process valid payment"));
}

#[test]
fn test_import_resolved_path() {
    let config = EmbeddingTextConfig::default();
    let builder = EmbeddingTextBuilder::new(config);

    let mut node = make_test_node();
    node.kind = NodeKind::Import;
    node.name = "PaymentService".to_string();

    let enrichment = NodeEnrichment {
        resolved_import_path: Some("src/services/payment.ts".to_string()),
        ..Default::default()
    };

    let text = builder.build_text(&node, &GraphContext::default(), &enrichment);

    assert!(text.contains("resolves to: src/services/payment.ts"));
}

#[test]
fn test_code_snippet_in_embedding() {
    let config = EmbeddingTextConfig {
        max_snippet_lines: 5,
        ..Default::default()
    };
    let builder = EmbeddingTextBuilder::new(config);

    let node = make_test_node();
    let snippet = (1..=10).map(|i| format!("  line{}", i)).collect::<Vec<_>>().join("\n");
    let enrichment = NodeEnrichment {
        code_snippet: Some(snippet),
        ..Default::default()
    };

    let text = builder.build_text(&node, &GraphContext::default(), &enrichment);

    // Should have snippet but truncated to 5 lines
    assert!(text.contains("line1"));
    assert!(text.contains("line5"));
    // line6+ should not be present (or should have truncation indicator)
}
```

**Step 2-5:** Implement enrichment field rendering.

**Commit:**
```bash
git commit -m "feat(vectors): add LSP enrichment fields and code snippets to embedding text"
```

---

### Task 12: Add Package Name Derivation

**Files:**
- Modify: `crates/codegraph-vectors/src/text_builder.rs`

**Step 1: Write the failing test**

```rust
#[test]
fn test_package_name_from_enrichment() {
    let config = EmbeddingTextConfig::default();
    let builder = EmbeddingTextBuilder::new(config);

    let node = make_test_node();
    let enrichment = NodeEnrichment {
        package_name: Some("@myapp/payments".to_string()),
        ..Default::default()
    };

    let text = builder.build_text(&node, &GraphContext::default(), &enrichment);

    assert!(text.contains("package: @myapp/payments"));
}

#[test]
fn test_package_name_fallback_to_directory() {
    let config = EmbeddingTextConfig::default();
    let builder = EmbeddingTextBuilder::new(config);

    let mut node = make_test_node();
    node.file_path = "src/services/payment/handler.ts".to_string();

    // No package_name in enrichment - should derive from path
    let enrichment = NodeEnrichment::default();

    let text = builder.build_text(&node, &GraphContext::default(), &enrichment);

    assert!(text.contains("package: src/services/payment"));
}
```

**Step 2-5:** Implement package derivation.

**Commit:**
```bash
git commit -m "feat(vectors): add package name with manifest-first, directory fallback (M2)"
```

---

### Task 13: Integrate Text Builder with Embedder

**Files:**
- Modify: `crates/codegraph-vectors/src/lib.rs`

**Step 1: Write the failing test**

```rust
#[test]
fn test_full_embedding_with_text_builder() {
    let text_config = EmbeddingTextConfig::default();
    let text_builder = EmbeddingTextBuilder::new(text_config);

    let embedder_config = EmbedderConfig::default();
    let mut embedder = TextEmbedder::new(embedder_config);

    let node = make_test_node();
    let context = GraphContext::default();
    let enrichment = NodeEnrichment::default();

    // Build text
    let text = text_builder.build_text(&node, &context, &enrichment);

    // Embed
    let embedding = embedder.embed(&text).unwrap();

    // Should have correct dimension
    assert_eq!(embedding.len(), 768); // nomic dimension (or mock dimension)
}
```

**Step 2-5:** Re-export text_builder types, ensure integration.

**Commit:**
```bash
git commit -m "feat(vectors): integrate text builder with embedder, export public API"
```

---

### Task 14: Add Embedding Version Tracking

**Files:**
- Modify: `crates/codegraph-vectors/src/text_builder.rs`
- Modify: `crates/codegraph-core/src/embedding.rs` (create if needed)

**Step 1: Write the failing test**

```rust
#[test]
fn test_embedding_version_hash() {
    let config1 = EmbeddingTextConfig::default();
    let config2 = EmbeddingTextConfig {
        max_tokens: 3000,
        ..Default::default()
    };

    let hash1 = EmbeddingTextConfig::version_hash(&config1);
    let hash2 = EmbeddingTextConfig::version_hash(&config2);

    // Different configs should have different hashes
    assert_ne!(hash1, hash2);

    // Same config should have same hash
    let hash1_again = EmbeddingTextConfig::version_hash(&config1);
    assert_eq!(hash1, hash1_again);
}
```

**Step 2-5:** Implement version hashing for re-embed detection (I13).

**Commit:**
```bash
git commit -m "feat(vectors): add embedding config version hash for re-embed detection (I13)"
```

---

## Milestone 3: Graph Context (Tasks 15-20)

### Task 15: Add Graph Context Query Methods (G4, G5)

**Files:**
- Modify: `crates/codegraph-graph/src/traversal.rs`

**Step 1: Write the failing test**

```rust
#[test]
fn test_get_callees_for_embedding() {
    let db = DatabaseConnection::open_in_memory().unwrap();
    setup_test_graph(&db); // Helper that creates nodes and edges

    let mut queries = QueryBuilder::new(db.conn()).unwrap();
    let mut traverser = GraphTraverser::new(db.conn(), &mut queries);

    // fn_a calls fn_b, fn_c, fn_d
    let callees = traverser.get_callees_for_embedding("fn_a", 10).unwrap();

    assert_eq!(callees.len(), 3);
    // Should be names, not IDs
    assert!(callees.contains(&"fn_b".to_string()));
}

#[test]
fn test_direct_only_no_transitive() {
    // G4: Direct relationships only, no transitive
    let db = DatabaseConnection::open_in_memory().unwrap();
    setup_chain_graph(&db); // fn_a -> fn_b -> fn_c

    let mut queries = QueryBuilder::new(db.conn()).unwrap();
    let mut traverser = GraphTraverser::new(db.conn(), &mut queries);

    let callees = traverser.get_callees_for_embedding("fn_a", 10).unwrap();

    // Should only get direct callee fn_b, NOT transitive fn_c
    assert_eq!(callees.len(), 1);
    assert!(callees.contains(&"fn_b".to_string()));
    assert!(!callees.contains(&"fn_c".to_string()));
}

#[test]
fn test_cycle_handling_safe() {
    // G5: Direct-only (G4) means cycles don't cause infinite loops
    let db = DatabaseConnection::open_in_memory().unwrap();
    setup_cyclic_graph(&db); // fn_a -> fn_b -> fn_c -> fn_a (cycle!)

    let mut queries = QueryBuilder::new(db.conn()).unwrap();
    let mut traverser = GraphTraverser::new(db.conn(), &mut queries);

    // This should NOT hang or panic - direct-only makes it safe
    let callees_a = traverser.get_callees_for_embedding("fn_a", 10).unwrap();
    let callees_b = traverser.get_callees_for_embedding("fn_b", 10).unwrap();
    let callees_c = traverser.get_callees_for_embedding("fn_c", 10).unwrap();

    // Each node only sees its direct callee
    assert_eq!(callees_a.len(), 1);
    assert!(callees_a.contains(&"fn_b".to_string()));
    assert_eq!(callees_b.len(), 1);
    assert!(callees_b.contains(&"fn_c".to_string()));
    assert_eq!(callees_c.len(), 1);
    assert!(callees_c.contains(&"fn_a".to_string())); // Cycle edge, but safe
}

fn setup_test_graph(db: &DatabaseConnection) {
    let queries = QueryBuilder::new(db.conn()).unwrap();

    let nodes = vec![
        Node::new("fn_a", NodeKind::Function, "fn_a", "test::fn_a", "test.rs", Language::Rust, 1, 5),
        Node::new("fn_b", NodeKind::Function, "fn_b", "test::fn_b", "test.rs", Language::Rust, 10, 15),
        Node::new("fn_c", NodeKind::Function, "fn_c", "test::fn_c", "test.rs", Language::Rust, 20, 25),
        Node::new("fn_d", NodeKind::Function, "fn_d", "test::fn_d", "test.rs", Language::Rust, 30, 35),
    ];

    for node in &nodes {
        queries.insert_node(db.conn(), node).unwrap();
    }

    // Create call edges: fn_a -> fn_b, fn_c, fn_d
    let edges = vec![
        Edge::new(NodeId::new("fn_a"), NodeId::new("fn_b"), EdgeKind::Calls),
        Edge::new(NodeId::new("fn_a"), NodeId::new("fn_c"), EdgeKind::Calls),
        Edge::new(NodeId::new("fn_a"), NodeId::new("fn_d"), EdgeKind::Calls),
    ];

    for edge in &edges {
        queries.insert_edge(db.conn(), edge).unwrap();
    }
}

fn setup_chain_graph(db: &DatabaseConnection) {
    let queries = QueryBuilder::new(db.conn()).unwrap();

    let nodes = vec![
        Node::new("fn_a", NodeKind::Function, "fn_a", "test::fn_a", "test.rs", Language::Rust, 1, 5),
        Node::new("fn_b", NodeKind::Function, "fn_b", "test::fn_b", "test.rs", Language::Rust, 10, 15),
        Node::new("fn_c", NodeKind::Function, "fn_c", "test::fn_c", "test.rs", Language::Rust, 20, 25),
    ];

    for node in &nodes {
        queries.insert_node(db.conn(), node).unwrap();
    }

    // Chain: fn_a -> fn_b -> fn_c
    queries.insert_edge(db.conn(), &Edge::new(NodeId::new("fn_a"), NodeId::new("fn_b"), EdgeKind::Calls)).unwrap();
    queries.insert_edge(db.conn(), &Edge::new(NodeId::new("fn_b"), NodeId::new("fn_c"), EdgeKind::Calls)).unwrap();
}

fn setup_cyclic_graph(db: &DatabaseConnection) {
    let queries = QueryBuilder::new(db.conn()).unwrap();

    let nodes = vec![
        Node::new("fn_a", NodeKind::Function, "fn_a", "test::fn_a", "test.rs", Language::Rust, 1, 5),
        Node::new("fn_b", NodeKind::Function, "fn_b", "test::fn_b", "test.rs", Language::Rust, 10, 15),
        Node::new("fn_c", NodeKind::Function, "fn_c", "test::fn_c", "test.rs", Language::Rust, 20, 25),
    ];

    for node in &nodes {
        queries.insert_node(db.conn(), node).unwrap();
    }

    // Cycle: fn_a -> fn_b -> fn_c -> fn_a
    queries.insert_edge(db.conn(), &Edge::new(NodeId::new("fn_a"), NodeId::new("fn_b"), EdgeKind::Calls)).unwrap();
    queries.insert_edge(db.conn(), &Edge::new(NodeId::new("fn_b"), NodeId::new("fn_c"), EdgeKind::Calls)).unwrap();
    queries.insert_edge(db.conn(), &Edge::new(NodeId::new("fn_c"), NodeId::new("fn_a"), EdgeKind::Calls)).unwrap();
}
```

**Step 2-5:** Implement using existing `get_callees()` method, extract names. Direct-only query naturally handles cycles (G5).

**Commit:**
```bash
git commit -m "feat(graph): add callee/caller queries for embedding (G4 direct-only, G5 cycle-safe)"
```

---

### Task 16: Add Sibling Query

**Files:**
- Modify: `crates/codegraph-graph/src/traversal.rs`

**Step 1-5:** Similar pattern - find parent via Contains edge, get other children.

**Commit:**
```bash
git commit -m "feat(graph): add sibling query for embedding context"
```

---

### Task 17: Add Inheritance Query

**Files:**
- Modify: `crates/codegraph-graph/src/traversal.rs`

**Step 1-5:** Query Implements and Extends edges.

**Commit:**
```bash
git commit -m "feat(graph): add implements/extends query for class context"
```

---

### Task 18: Add Priority Sorting for Graph Lists (G3, G6)

**Files:**
- Modify: `crates/codegraph-graph/src/traversal.rs`

**Step 1: Write the failing test**

```rust
#[test]
fn test_callees_sorted_decorated_first() {
    let db = DatabaseConnection::open_in_memory().unwrap();
    setup_graph_with_decorated_nodes(&db);

    let mut queries = QueryBuilder::new(db.conn()).unwrap();
    let mut traverser = GraphTraverser::new(db.conn(), &mut queries);

    let callees = traverser.get_callees_for_embedding("caller", 10).unwrap();

    // G3: Decorated nodes should come first
    // fn_decorated has @Controller, fn_plain has no decorators
    assert_eq!(callees[0], "fn_decorated");
}

#[test]
fn test_callees_sorted_by_call_frequency() {
    // G3, G6: After decorated, sort by call frequency (counted at query time)
    let db = DatabaseConnection::open_in_memory().unwrap();
    setup_graph_with_call_counts(&db);

    // fn_popular is called by 10 other functions
    // fn_rare is called by 1 function
    // Neither is decorated

    let mut queries = QueryBuilder::new(db.conn()).unwrap();
    let mut traverser = GraphTraverser::new(db.conn(), &mut queries);

    let callees = traverser.get_callees_for_embedding("caller", 10).unwrap();

    // More frequently called functions come first
    let popular_idx = callees.iter().position(|c| c == "fn_popular").unwrap();
    let rare_idx = callees.iter().position(|c| c == "fn_rare").unwrap();
    assert!(popular_idx < rare_idx, "Popular functions should rank higher");
}

#[test]
fn test_callees_alphabetical_tiebreaker() {
    // G3: Same decoration status and frequency -> alphabetical
    let db = DatabaseConnection::open_in_memory().unwrap();
    setup_graph_with_same_priority(&db);

    let mut queries = QueryBuilder::new(db.conn()).unwrap();
    let mut traverser = GraphTraverser::new(db.conn(), &mut queries);

    let callees = traverser.get_callees_for_embedding("caller", 10).unwrap();

    // Alphabetical when tied
    assert_eq!(callees[0], "fn_alpha");
    assert_eq!(callees[1], "fn_beta");
}

fn setup_graph_with_call_counts(db: &DatabaseConnection) {
    let queries = QueryBuilder::new(db.conn()).unwrap();

    // fn_popular - called by many
    let popular = Node::new("fn_popular", NodeKind::Function, "fn_popular", "test::fn_popular", "test.rs", Language::Rust, 1, 5);
    queries.insert_node(db.conn(), &popular).unwrap();

    // fn_rare - called by few
    let rare = Node::new("fn_rare", NodeKind::Function, "fn_rare", "test::fn_rare", "test.rs", Language::Rust, 10, 15);
    queries.insert_node(db.conn(), &rare).unwrap();

    // Create many callers for fn_popular (G6: count at query time)
    for i in 0..10 {
        let caller = Node::new(format!("caller_{}", i), NodeKind::Function, format!("caller_{}", i), format!("test::caller_{}", i), "test.rs", Language::Rust, 20 + i, 25 + i);
        queries.insert_node(db.conn(), &caller).unwrap();
        queries.insert_edge(db.conn(), &Edge::new(NodeId::new(format!("caller_{}", i)), NodeId::new("fn_popular"), EdgeKind::Calls)).unwrap();
    }

    // Only 1 caller for fn_rare
    queries.insert_edge(db.conn(), &Edge::new(NodeId::new("caller_0"), NodeId::new("fn_rare"), EdgeKind::Calls)).unwrap();
}
```

**Step 2-5:** Implement priority sorting: decorated → call frequency → alphabetical.

```rust
/// Sort callees by priority (G3): decorated first, then by call frequency (G6), then alphabetical
fn sort_by_priority(nodes: &mut [Node], conn: &Connection) {
    // Count incoming calls for each node (G6: count at query time)
    let call_counts: HashMap<String, usize> = nodes.iter()
        .map(|n| {
            let count = count_incoming_calls(conn, &n.id.0).unwrap_or(0);
            (n.id.0.clone(), count)
        })
        .collect();

    nodes.sort_by(|a, b| {
        // 1. Decorated first
        let a_decorated = !a.decorators.is_empty();
        let b_decorated = !b.decorators.is_empty();
        if a_decorated != b_decorated {
            return b_decorated.cmp(&a_decorated);
        }

        // 2. Higher call frequency (G6)
        let a_freq = call_counts.get(&a.id.0).unwrap_or(&0);
        let b_freq = call_counts.get(&b.id.0).unwrap_or(&0);
        if a_freq != b_freq {
            return b_freq.cmp(a_freq);
        }

        // 3. Alphabetical tiebreaker
        a.name.cmp(&b.name)
    });
}

fn count_incoming_calls(conn: &Connection, node_id: &str) -> Result<usize, DbError> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM edges WHERE target = ?1 AND kind = 'calls'",
        params![node_id],
        |row| row.get(0),
    )?;
    Ok(count as usize)
}
```

**Commit:**
```bash
git commit -m "feat(graph): add priority sorting (decorated → frequency → alpha) for graph lists (G3, G6)"
```

---

### Task 19: Implement Batched Graph Queries (B7)

**Files:**
- Create: `crates/codegraph-graph/src/batch.rs`
- Modify: `crates/codegraph-graph/src/lib.rs`

**Step 1: Write the failing test**

```rust
#[test]
fn test_batch_graph_context() {
    let db = DatabaseConnection::open_in_memory().unwrap();
    setup_large_test_graph(&db, 100);

    let config = EmbeddingTextConfig::default();
    let batch = BatchGraphQuerier::new(db.conn());

    let node_ids: Vec<&str> = (0..100).map(|i| format!("node_{}", i)).collect();
    let node_refs: Vec<&str> = node_ids.iter().map(|s| s.as_str()).collect();

    let contexts = batch.get_graph_contexts(&node_refs, &config).unwrap();

    assert_eq!(contexts.len(), 100);
}

#[test]
fn test_batch_query_uses_cte() {
    // This is more of an implementation detail test
    // Verify that we don't make N queries for N nodes
    let db = DatabaseConnection::open_in_memory().unwrap();
    setup_large_test_graph(&db, 1000);

    let config = EmbeddingTextConfig::default();
    let batch = BatchGraphQuerier::new(db.conn());

    let node_ids: Vec<String> = (0..1000).map(|i| format!("node_{}", i)).collect();
    let node_refs: Vec<&str> = node_ids.iter().map(|s| s.as_str()).collect();

    let start = std::time::Instant::now();
    let contexts = batch.get_graph_contexts(&node_refs, &config).unwrap();
    let elapsed = start.elapsed();

    assert_eq!(contexts.len(), 1000);
    // Batch should be fast - under 500ms for 1000 nodes
    assert!(elapsed.as_millis() < 500, "Batch took too long: {:?}", elapsed);
}
```

**Step 2-5:** Implement CTE-based batch queries.

**Commit:**
```bash
git commit -m "feat(graph): implement batched graph context queries using CTEs (B7)"
```

---

### Task 20: Integrate Graph Context with Embedding Pipeline

**Files:**
- Create: `crates/codegraph-core/src/embedding.rs`

**Step 1-5:** Wire up graph queries to embedding text generation.

**Commit:**
```bash
git commit -m "feat(core): integrate graph context into embedding pipeline"
```

---

## Milestone 4: LSP Foundation (Tasks 21-30)

### Task 21: Create codegraph-lsp Crate

**Files:**
- Create: `crates/codegraph-lsp/Cargo.toml`
- Create: `crates/codegraph-lsp/src/lib.rs`
- Modify: `Cargo.toml` (workspace)

**Step 1: Create crate structure**

```toml
# crates/codegraph-lsp/Cargo.toml
[package]
name = "codegraph-lsp"
version.workspace = true
edition.workspace = true
license.workspace = true
rust-version.workspace = true

[dependencies]
codegraph-types = { path = "../codegraph-types" }
codegraph-db = { path = "../codegraph-db" }

# LSP client
tower-lsp = "0.20"
lsp-types = "0.95"

# Async runtime
tokio = { version = "1", features = ["full", "process", "time"] }
async-trait = "0.1"

# Serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"

# Error handling
thiserror = "1"

# Logging
log = "0.4"

[dev-dependencies]
tempfile = { workspace = true }
```

```rust
// crates/codegraph-lsp/src/lib.rs
//! LSP enrichment for CodeGraph
//!
//! Provides language server integration for enriching nodes with:
//! - Inferred types (hover)
//! - Resolved import paths (definition)

pub mod client;
pub mod encoding;
pub mod enricher;
pub mod error;
pub mod lifecycle;

// Language-specific enrichers
pub mod typescript;
// pub mod dart;  // Task 31
// pub mod rust_analyzer;  // Task 32

pub use enricher::{LspEnricher, HoverResult, DefinitionResult};
pub use error::LspError;
pub use lifecycle::LspServerManager;
```

**Step 2: Update workspace Cargo.toml**

```toml
[workspace]
members = [
    "crates/codegraph-cli",
    "crates/codegraph-core",
    "crates/codegraph-db",
    "crates/codegraph-extraction",
    "crates/codegraph-graph",
    "crates/codegraph-lsp",  # Add this
    "crates/codegraph-mcp",
    "crates/codegraph-resolution",
    "crates/codegraph-sync",
    "crates/codegraph-types",
    "crates/codegraph-vectors",
]
```

**Step 3: Write basic test**

```rust
#[test]
fn test_crate_compiles() {
    // Just verify the crate structure works
    use codegraph_lsp::LspError;
    let _err: Option<LspError> = None;
}
```

**Step 4-5:** Build and commit.

**Commit:**
```bash
git commit -m "feat(lsp): create codegraph-lsp crate with dependencies"
```

---

### Task 22: Define LspEnricher Trait

**Files:**
- Create: `crates/codegraph-lsp/src/enricher.rs`

**Step 1: Write the failing test**

```rust
#[test]
fn test_lsp_enricher_trait_is_object_safe() {
    // Trait should be usable as trait object
    fn accepts_enricher(_e: &dyn LspEnricher) {}

    // This compiles = trait is object safe
}
```

**Step 2-5:** Define trait.

```rust
// crates/codegraph-lsp/src/enricher.rs
use crate::error::LspError;
use async_trait::async_trait;
use codegraph_types::Language;
use std::path::Path;

/// Result of an LSP hover query
#[derive(Debug, Clone)]
pub struct HoverResult {
    /// Inferred type from hover contents
    pub inferred_type: Option<String>,
    /// Documentation from hover
    pub documentation: Option<String>,
}

/// Result of an LSP definition query
#[derive(Debug, Clone)]
pub struct DefinitionResult {
    /// File path of the definition
    pub file_path: String,
    /// Line number (0-indexed)
    pub line: u32,
    /// Column number (0-indexed)
    pub column: u32,
}

/// LSP enricher trait for language-specific implementations
#[async_trait]
pub trait LspEnricher: Send + Sync {
    /// Start the LSP server for a workspace
    async fn start(&mut self, workspace_root: &Path) -> Result<(), LspError>;

    /// Query hover information at a position
    async fn hover(
        &self,
        file: &Path,
        line: u32,
        column: u32,
    ) -> Result<Option<HoverResult>, LspError>;

    /// Query definition at a position
    async fn definition(
        &self,
        file: &Path,
        line: u32,
        column: u32,
    ) -> Result<Option<DefinitionResult>, LspError>;

    /// Shutdown the LSP server
    async fn shutdown(&mut self) -> Result<(), LspError>;

    /// Check if server is ready (workspace fully initialized)
    fn is_ready(&self) -> bool;

    /// Get the language this enricher handles
    fn language(&self) -> Language;
}
```

**Commit:**
```bash
git commit -m "feat(lsp): define LspEnricher trait for language-specific implementations"
```

---

### Task 23: Add LSP Error Types

**Files:**
- Create: `crates/codegraph-lsp/src/error.rs`

**Commit:**
```bash
git commit -m "feat(lsp): add LspError types"
```

---

### Task 24: Add UTF-16 Position Encoding (I9)

**Files:**
- Create: `crates/codegraph-lsp/src/encoding.rs`

**Step 1: Write the failing test**

```rust
#[test]
fn test_byte_to_utf16_ascii() {
    let text = "Hello World";
    let byte_offset = 6; // Start of "World"
    let utf16_offset = byte_to_utf16(text, byte_offset);
    assert_eq!(utf16_offset, 6); // Same for ASCII
}

#[test]
fn test_byte_to_utf16_emoji() {
    // "Hello 🌍 World" - emoji is 4 bytes but 2 UTF-16 code units
    let text = "Hello 🌍 World";
    let byte_offset = 11; // Start of "World" (after "Hello " + 4-byte emoji + " ")
    let utf16_offset = byte_to_utf16(text, byte_offset);
    // "Hello " = 6, "🌍" = 2 (surrogate pair), " " = 1 = 9
    assert_eq!(utf16_offset, 9);
}

#[test]
fn test_utf16_to_byte_roundtrip() {
    let text = "Hello 🌍 World 你好";

    for (byte_idx, _) in text.char_indices() {
        let utf16 = byte_to_utf16(text, byte_idx);
        let back = utf16_to_byte(text, utf16);
        assert_eq!(back, byte_idx, "Roundtrip failed at byte {}", byte_idx);
    }
}
```

**Step 2-5:** Implement encoding conversions.

**Commit:**
```bash
git commit -m "feat(lsp): add UTF-16 position encoding conversion (I9)"
```

---

### Task 25: Add LSP Client Base

**Files:**
- Create: `crates/codegraph-lsp/src/client.rs`

**Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_lsp_client_send_request() {
        // Mock server that echoes back
        let (mut client, _server) = create_mock_client_server();

        let params = lsp_types::HoverParams {
            text_document_position_params: lsp_types::TextDocumentPositionParams {
                text_document: lsp_types::TextDocumentIdentifier {
                    uri: lsp_types::Url::parse("file:///test.ts").unwrap(),
                },
                position: lsp_types::Position { line: 0, character: 0 },
            },
            work_done_progress_params: Default::default(),
        };

        let result = client.hover(params).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_lsp_client_handles_error_response() {
        let (mut client, _server) = create_mock_client_server();

        // Request for non-existent file
        let params = lsp_types::HoverParams {
            text_document_position_params: lsp_types::TextDocumentPositionParams {
                text_document: lsp_types::TextDocumentIdentifier {
                    uri: lsp_types::Url::parse("file:///nonexistent.ts").unwrap(),
                },
                position: lsp_types::Position { line: 0, character: 0 },
            },
            work_done_progress_params: Default::default(),
        };

        let result = client.hover(params).await;
        // Should return None, not error (file not found is valid LSP response)
        assert!(result.is_ok());
    }
}
```

**Step 2-5:** Implement base LSP client using async-lsp.

```rust
// crates/codegraph-lsp/src/client.rs
use async_lsp::{LanguageClient, LspService, MainLoop};
use lsp_types::*;
use std::process::Stdio;
use tokio::process::Command;
use crate::error::LspError;

/// LSP client wrapper for communication with language servers
pub struct LspClient {
    service: LspService,
    main_loop: Option<MainLoop>,
}

impl LspClient {
    /// Spawn a new LSP server process and connect
    pub async fn spawn(command: &str, args: &[String], root_uri: Url) -> Result<Self, LspError> {
        let mut child = Command::new(command)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| LspError::SpawnFailed(e.to_string()))?;

        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();

        let (service, main_loop) = LspService::build(stdin, stdout)
            .finish();

        Ok(Self { service, main_loop: Some(main_loop) })
    }

    /// Send hover request
    pub async fn hover(&mut self, params: HoverParams) -> Result<Option<Hover>, LspError> {
        self.service.hover(params).await
            .map_err(|e| LspError::RequestFailed(e.to_string()))
    }

    /// Send go-to-definition request
    pub async fn definition(&mut self, params: GotoDefinitionParams) -> Result<Option<GotoDefinitionResponse>, LspError> {
        self.service.definition(params).await
            .map_err(|e| LspError::RequestFailed(e.to_string()))
    }

    /// Shutdown the server
    pub async fn shutdown(&mut self) -> Result<(), LspError> {
        self.service.shutdown().await
            .map_err(|e| LspError::ShutdownFailed(e.to_string()))?;
        self.service.exit().await;
        Ok(())
    }
}
```

**Commit:**
```bash
git commit -m "feat(lsp): add base LSP client using async-lsp"
```

---

### Task 26: Add Server Lifecycle Management (L5)

**Files:**
- Create: `crates/codegraph-lsp/src/lifecycle.rs`

**Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use std::fs;

    fn setup_test_project() -> TempDir {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("package.json"), r#"{"name": "test"}"#).unwrap();
        fs::write(temp.path().join("tsconfig.json"), r#"{"compilerOptions": {}}"#).unwrap();
        fs::write(temp.path().join("index.ts"), "export const x = 1;").unwrap();
        temp
    }

    #[tokio::test]
    async fn test_lazy_spawn_not_started_until_needed() {
        let config = LspServerConfig {
            enabled: true,
            server: "typescript-language-server".to_string(),
            args: vec!["--stdio".to_string()],
        };

        let manager = LspServerManager::new(config);

        // Not spawned yet
        assert!(!manager.is_running());
    }

    #[tokio::test]
    #[ignore] // Requires typescript-language-server
    async fn test_lazy_spawn_starts_on_first_request() {
        let temp = setup_test_project();
        let config = LspServerConfig {
            enabled: true,
            server: "typescript-language-server".to_string(),
            args: vec!["--stdio".to_string()],
        };

        let mut manager = LspServerManager::new(config);

        // First request triggers spawn
        manager.ensure_started(temp.path()).await.unwrap();
        assert!(manager.is_running());
    }

    #[tokio::test]
    #[ignore] // Requires typescript-language-server
    async fn test_keep_alive_during_indexing() {
        let temp = setup_test_project();
        let config = LspServerConfig {
            enabled: true,
            server: "typescript-language-server".to_string(),
            args: vec!["--stdio".to_string()],
        };

        let mut manager = LspServerManager::new(config);
        manager.ensure_started(temp.path()).await.unwrap();

        // Should stay alive during batch operations
        for _ in 0..10 {
            assert!(manager.is_running());
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }

    #[tokio::test]
    #[ignore] // Requires typescript-language-server
    async fn test_shutdown_after_indexing() {
        let temp = setup_test_project();
        let config = LspServerConfig {
            enabled: true,
            server: "typescript-language-server".to_string(),
            args: vec!["--stdio".to_string()],
        };

        let mut manager = LspServerManager::new(config);
        manager.ensure_started(temp.path()).await.unwrap();
        assert!(manager.is_running());

        manager.shutdown().await.unwrap();
        assert!(!manager.is_running());
    }
}
```

**Step 2-5:** Implement server lifecycle.

```rust
// crates/codegraph-lsp/src/lifecycle.rs
use crate::client::LspClient;
use crate::error::LspError;
use codegraph_types::LspServerConfig;
use std::path::Path;
use lsp_types::Url;

/// Manages LSP server lifecycle (L5: lazy spawn, keep-alive, shutdown)
pub struct LspServerManager {
    config: LspServerConfig,
    client: Option<LspClient>,
    root_uri: Option<Url>,
}

impl LspServerManager {
    pub fn new(config: LspServerConfig) -> Self {
        Self {
            config,
            client: None,
            root_uri: None,
        }
    }

    /// Check if server is currently running
    pub fn is_running(&self) -> bool {
        self.client.is_some()
    }

    /// Ensure server is started (lazy spawn)
    pub async fn ensure_started(&mut self, project_root: &Path) -> Result<&mut LspClient, LspError> {
        if self.client.is_none() {
            let root_uri = Url::from_file_path(project_root)
                .map_err(|_| LspError::InvalidPath(project_root.display().to_string()))?;

            let client = LspClient::spawn(
                &self.config.server,
                &self.config.args,
                root_uri.clone(),
            ).await?;

            self.client = Some(client);
            self.root_uri = Some(root_uri);
        }

        Ok(self.client.as_mut().unwrap())
    }

    /// Get client if running
    pub fn client(&mut self) -> Option<&mut LspClient> {
        self.client.as_mut()
    }

    /// Shutdown the server
    pub async fn shutdown(&mut self) -> Result<(), LspError> {
        if let Some(mut client) = self.client.take() {
            client.shutdown().await?;
        }
        self.root_uri = None;
        Ok(())
    }
}
```

**Commit:**
```bash
git commit -m "feat(lsp): add server lifecycle management (lazy spawn, keep-alive, shutdown) (L5)"
```

---

### Task 27: Add Workspace Initialization Waiting (I10)

**Files:**
- Modify: `crates/codegraph-lsp/src/lifecycle.rs`

**Step 1-5:** Wait for workspace/didChangeConfiguration or indexing complete.

**Commit:**
```bash
git commit -m "feat(lsp): add workspace initialization waiting with timeout (I10)"
```

---

### Task 28: Add Error Handling and Retry (I6, I7)

**Files:**
- Modify: `crates/codegraph-lsp/src/lifecycle.rs`
- Create: `crates/codegraph-lsp/src/batch.rs`

**Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_spawn_retry_with_backoff() {
        let config = LspServerConfig {
            enabled: true,
            server: "nonexistent-server-that-will-fail".to_string(),
            args: vec![],
        };

        let mut manager = LspServerManager::new(config);
        let result = manager.ensure_started_with_retry(Path::new("/tmp"), 3).await;

        // Should fail after 3 retries
        assert!(result.is_err());
        match result.unwrap_err() {
            LspError::SpawnFailed(msg) => assert!(msg.contains("after 3 retries")),
            e => panic!("Unexpected error: {:?}", e),
        }
    }

    #[tokio::test]
    async fn test_per_file_error_skips_and_continues() {
        let mut enricher = MockEnricher::new();
        enricher.fail_on_file("bad_syntax.ts");

        let files = vec!["good1.ts", "bad_syntax.ts", "good2.ts"];
        let results = enricher.enrich_files(&files).await.unwrap();

        // Should have 2 results (bad_syntax.ts skipped)
        assert_eq!(results.len(), 2);
        assert!(results.iter().any(|r| r.file_path == "good1.ts"));
        assert!(results.iter().any(|r| r.file_path == "good2.ts"));
        // bad_syntax.ts was skipped with warning
    }

    #[tokio::test]
    async fn test_server_crash_restarts_and_continues() {
        let mut enricher = MockEnricher::new();
        enricher.crash_on_file("crash_trigger.ts");

        let files = vec!["a.ts", "crash_trigger.ts", "b.ts"];
        let results = enricher.enrich_files(&files).await.unwrap();

        // Server should restart, continue with remaining files
        assert!(results.iter().any(|r| r.file_path == "a.ts"));
        assert!(results.iter().any(|r| r.file_path == "b.ts"));
        assert_eq!(enricher.restart_count(), 1);
    }

    #[tokio::test]
    async fn test_degrade_mode_continues_without_lsp() {
        let config = EnrichmentConfig {
            on_lsp_unavailable: LspUnavailableAction::Degrade,
            ..Default::default()
        };

        let mut enricher = LspEnricherBatch::new(config);
        // LSP server unavailable
        enricher.set_lsp_available(false);

        let files = vec!["test.ts"];
        let results = enricher.enrich_files(&files).await.unwrap();

        // Should return empty enrichment, not error
        assert_eq!(results.len(), 1);
        assert!(results[0].inferred_type.is_none());
    }

    #[tokio::test]
    async fn test_fail_mode_errors_without_lsp() {
        let config = EnrichmentConfig {
            on_lsp_unavailable: LspUnavailableAction::Fail,
            ..Default::default()
        };

        let mut enricher = LspEnricherBatch::new(config);
        enricher.set_lsp_available(false);

        let files = vec!["test.ts"];
        let result = enricher.enrich_files(&files).await;

        // Should fail
        assert!(result.is_err());
    }
}
```

**Step 2-5:** Implement error handling and retry logic.

```rust
// crates/codegraph-lsp/src/batch.rs
use crate::error::LspError;
use crate::lifecycle::LspServerManager;
use codegraph_types::{EnrichmentConfig, LspUnavailableAction, Node};
use std::path::Path;
use std::time::Duration;

/// Batch LSP enrichment with error handling (I6, I7)
pub struct LspEnricherBatch {
    config: EnrichmentConfig,
    manager: Option<LspServerManager>,
    restart_count: usize,
}

/// Result of enriching a single file
#[derive(Debug, Clone)]
pub struct FileEnrichmentResult {
    pub file_path: String,
    pub inferred_type: Option<String>,
    pub resolved_import_path: Option<String>,
}

impl LspEnricherBatch {
    pub fn new(config: EnrichmentConfig) -> Self {
        Self {
            config,
            manager: None,
            restart_count: 0,
        }
    }

    /// Enrich multiple files with per-file error handling (I7)
    pub async fn enrich_files(&mut self, files: &[&str]) -> Result<Vec<FileEnrichmentResult>, LspError> {
        let mut results = Vec::new();

        for file in files {
            match self.enrich_file(file).await {
                Ok(result) => results.push(result),
                Err(LspError::ServerCrashed) => {
                    log::warn!("LSP server crashed on {}, restarting...", file);
                    self.restart_server().await?;
                    self.restart_count += 1;
                    // Skip this file, continue with next
                }
                Err(e) => {
                    log::warn!("LSP error on {}: {}, skipping file", file, e);
                    // Skip this file, continue with next (I7)
                }
            }
        }

        Ok(results)
    }

    /// Enrich a single file
    async fn enrich_file(&mut self, file: &str) -> Result<FileEnrichmentResult, LspError> {
        // ... implementation
        Ok(FileEnrichmentResult {
            file_path: file.to_string(),
            inferred_type: None,
            resolved_import_path: None,
        })
    }

    /// Restart the LSP server after crash
    async fn restart_server(&mut self) -> Result<(), LspError> {
        if let Some(ref mut manager) = self.manager {
            manager.shutdown().await.ok(); // Ignore shutdown errors
        }
        // Will lazy-restart on next request
        Ok(())
    }

    pub fn restart_count(&self) -> usize {
        self.restart_count
    }
}

// Add to lifecycle.rs
impl LspServerManager {
    /// Ensure started with retry logic (I6: 3x with backoff)
    pub async fn ensure_started_with_retry(
        &mut self,
        project_root: &Path,
        max_retries: u32,
    ) -> Result<&mut LspClient, LspError> {
        let mut last_error = None;
        let mut delay = Duration::from_millis(100);

        for attempt in 1..=max_retries {
            match self.ensure_started(project_root).await {
                Ok(client) => return Ok(client),
                Err(e) => {
                    log::warn!("LSP spawn attempt {} failed: {}", attempt, e);
                    last_error = Some(e);
                    if attempt < max_retries {
                        tokio::time::sleep(delay).await;
                        delay *= 2; // Exponential backoff
                    }
                }
            }
        }

        Err(LspError::SpawnFailed(format!(
            "Failed to spawn LSP server after {} retries: {:?}",
            max_retries,
            last_error
        )))
    }
}
```

**Commit:**
```bash
git commit -m "feat(lsp): add retry logic (I6) and per-file error handling (I7)"
```

---

### Task 29: Implement TypeScript Enricher

**Files:**
- Create: `crates/codegraph-lsp/src/typescript.rs`

**Step 1: Write the failing test**

```rust
#[tokio::test]
#[ignore] // Requires typescript-language-server installed
async fn test_typescript_enricher_start() {
    use tempfile::TempDir;
    use std::fs;

    let temp = TempDir::new().unwrap();

    // Create minimal TS project
    fs::write(temp.path().join("package.json"), r#"{"name": "test"}"#).unwrap();
    fs::write(temp.path().join("tsconfig.json"), r#"{"compilerOptions": {}}"#).unwrap();
    fs::write(temp.path().join("index.ts"), "export const x: number = 42;").unwrap();

    let mut enricher = TypeScriptEnricher::new(LspServerConfig {
        enabled: true,
        server: "typescript-language-server".to_string(),
        args: vec!["--stdio".to_string()],
    });

    enricher.start(temp.path()).await.unwrap();
    assert!(enricher.is_ready());

    enricher.shutdown().await.unwrap();
}
```

**Step 2-5:** Implement TypeScript LSP client.

**Commit:**
```bash
git commit -m "feat(lsp): implement TypeScript LSP enricher"
```

---

### Task 30: Add Async/Sync Bridge

**Files:**
- Create: `crates/codegraph-lsp/src/sync_bridge.rs`

**Step 1: Write the failing test**

```rust
#[test]
fn test_sync_bridge_calls_async() {
    let bridge = LspSyncBridge::new();

    // This should work from sync code
    let result = bridge.block_on(async {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        42
    });

    assert_eq!(result, 42);
}
```

**Step 2-5:** Implement sync wrapper using `tokio::runtime::Runtime::block_on()`.

**Commit:**
```bash
git commit -m "feat(lsp): add sync bridge for calling async LSP from sync code"
```

---

## Milestone 5: LSP Languages (Tasks 31-34)

### Task 31: Implement Dart Enricher (L3)

**Files:**
- Create: `crates/codegraph-lsp/src/dart.rs`

**Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use std::fs;

    fn setup_dart_project() -> TempDir {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("pubspec.yaml"), "name: test_app\n").unwrap();
        fs::write(temp.path().join("lib/main.dart"), r#"
void main() {
  final greeting = getGreeting();
  print(greeting);
}

String getGreeting() => 'Hello, World!';
"#).unwrap();
        temp
    }

    #[tokio::test]
    #[ignore] // Requires dart analysis server
    async fn test_dart_enricher_hover() {
        let temp = setup_dart_project();
        let config = LspServerConfig {
            enabled: true,
            server: "dart".to_string(),
            args: vec!["language-server".to_string(), "--protocol=lsp".to_string()],
        };

        let mut enricher = DartEnricher::new(config);
        enricher.start(temp.path()).await.unwrap();

        let hover = enricher.hover(
            temp.path().join("lib/main.dart").to_str().unwrap(),
            5, // line of getGreeting()
            10,
        ).await.unwrap();

        assert!(hover.inferred_type.is_some());
        assert!(hover.inferred_type.unwrap().contains("String"));

        enricher.shutdown().await.unwrap();
    }
}
```

**Step 2-5:** Implement Dart LSP enricher following TypeScript pattern.

```rust
// crates/codegraph-lsp/src/dart.rs
use crate::client::LspClient;
use crate::enricher::{LspEnricher, HoverResult, DefinitionResult};
use crate::error::LspError;
use async_trait::async_trait;
use codegraph_types::{Language, LspServerConfig};
use std::path::Path;

pub struct DartEnricher {
    config: LspServerConfig,
    client: Option<LspClient>,
}

impl DartEnricher {
    pub fn new(config: LspServerConfig) -> Self {
        Self { config, client: None }
    }
}

#[async_trait]
impl LspEnricher for DartEnricher {
    fn language(&self) -> Language {
        Language::Dart
    }

    async fn start(&mut self, project_root: &Path) -> Result<(), LspError> {
        let root_uri = lsp_types::Url::from_file_path(project_root)
            .map_err(|_| LspError::InvalidPath(project_root.display().to_string()))?;

        self.client = Some(LspClient::spawn(
            &self.config.server,
            &self.config.args,
            root_uri,
        ).await?);

        Ok(())
    }

    async fn hover(&mut self, file: &str, line: u32, col: u32) -> Result<HoverResult, LspError> {
        let client = self.client.as_mut()
            .ok_or(LspError::NotStarted)?;

        let params = lsp_types::HoverParams {
            text_document_position_params: lsp_types::TextDocumentPositionParams {
                text_document: lsp_types::TextDocumentIdentifier {
                    uri: lsp_types::Url::from_file_path(file)
                        .map_err(|_| LspError::InvalidPath(file.to_string()))?,
                },
                position: lsp_types::Position { line, character: col },
            },
            work_done_progress_params: Default::default(),
        };

        let hover = client.hover(params).await?;
        Ok(HoverResult::from_lsp(hover))
    }

    async fn definition(&mut self, file: &str, line: u32, col: u32) -> Result<Option<DefinitionResult>, LspError> {
        // Similar to hover implementation
        Ok(None)
    }

    async fn shutdown(&mut self) -> Result<(), LspError> {
        if let Some(mut client) = self.client.take() {
            client.shutdown().await?;
        }
        Ok(())
    }

    fn is_ready(&self) -> bool {
        self.client.is_some()
    }
}
```

**Commit:**
```bash
git commit -m "feat(lsp): implement Dart LSP enricher (L3)"
```

---

### Task 32: Implement Rust Enricher (L3)

**Files:**
- Create: `crates/codegraph-lsp/src/rust_analyzer.rs`

**Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use std::fs;

    fn setup_rust_project() -> TempDir {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("Cargo.toml"), r#"
[package]
name = "test"
version = "0.1.0"
edition = "2021"
"#).unwrap();
        fs::create_dir_all(temp.path().join("src")).unwrap();
        fs::write(temp.path().join("src/lib.rs"), r#"
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}
"#).unwrap();
        temp
    }

    #[tokio::test]
    #[ignore] // Requires rust-analyzer
    async fn test_rust_enricher_hover() {
        let temp = setup_rust_project();
        let config = LspServerConfig {
            enabled: true,
            server: "rust-analyzer".to_string(),
            args: vec![],
        };

        let mut enricher = RustEnricher::new(config);
        enricher.start(temp.path()).await.unwrap();

        // Wait for rust-analyzer to index
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;

        let hover = enricher.hover(
            temp.path().join("src/lib.rs").to_str().unwrap(),
            1, // fn add line
            7, // position of 'add'
        ).await.unwrap();

        assert!(hover.inferred_type.is_some());
        assert!(hover.inferred_type.unwrap().contains("i32"));

        enricher.shutdown().await.unwrap();
    }
}
```

**Step 2-5:** Implement Rust enricher following same pattern as Dart.

**Commit:**
```bash
git commit -m "feat(lsp): implement Rust LSP enricher (L3)"
```

---

### Task 33: Add Multi-Instance LSP Pool (I5, I5a, I4)

**Files:**
- Create: `crates/codegraph-lsp/src/pool.rs`

**Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_pool_single_instance_default() {
        let config = EnrichmentConfig {
            lsp_instances: 1,
            ..Default::default()
        };

        let pool = LspPool::new(config);
        assert_eq!(pool.instance_count(), 1);
    }

    #[tokio::test]
    async fn test_pool_multiple_instances() {
        let config = EnrichmentConfig {
            lsp_instances: 4,
            ..Default::default()
        };

        let pool = LspPool::new(config);
        assert_eq!(pool.instance_count(), 4);
    }

    #[tokio::test]
    async fn test_work_stealing_queue_distribution() {
        let config = EnrichmentConfig {
            lsp_instances: 2,
            ..Default::default()
        };

        let mut pool = LspPool::new(config);

        // Submit 10 files
        let files: Vec<String> = (0..10).map(|i| format!("file{}.ts", i)).collect();
        let file_refs: Vec<&str> = files.iter().map(|s| s.as_str()).collect();

        pool.submit_batch(&file_refs).await;

        // Work should be distributed across instances
        let stats = pool.get_stats();
        assert!(stats.instance_0_processed > 0);
        assert!(stats.instance_1_processed > 0);
    }

    #[tokio::test]
    async fn test_blocking_execution_waits_for_completion() {
        // I4: Blocking execution - indexing waits for enrichment
        let config = EnrichmentConfig::default();
        let mut pool = LspPool::new(config);

        let files = vec!["a.ts", "b.ts", "c.ts"];
        let results = pool.enrich_all_blocking(&files).await.unwrap();

        // All files should be enriched before returning
        assert_eq!(results.len(), 3);
    }
}
```

**Step 2-5:** Implement LSP pool with work-stealing.

```rust
// crates/codegraph-lsp/src/pool.rs
use crate::batch::LspEnricherBatch;
use crate::error::LspError;
use codegraph_types::EnrichmentConfig;
use tokio::sync::mpsc;
use std::sync::Arc;

/// Multi-instance LSP pool with work-stealing (I5, I5a)
pub struct LspPool {
    config: EnrichmentConfig,
    instances: Vec<LspEnricherBatch>,
    work_queue: mpsc::Sender<String>,
}

#[derive(Debug, Default)]
pub struct PoolStats {
    pub instance_0_processed: usize,
    pub instance_1_processed: usize,
}

impl LspPool {
    pub fn new(config: EnrichmentConfig) -> Self {
        let count = config.lsp_instances as usize;
        let instances = (0..count)
            .map(|_| LspEnricherBatch::new(config.clone()))
            .collect();

        let (tx, _rx) = mpsc::channel(1000);

        Self {
            config,
            instances,
            work_queue: tx,
        }
    }

    pub fn instance_count(&self) -> usize {
        self.instances.len()
    }

    /// Enrich all files, blocking until complete (I4)
    pub async fn enrich_all_blocking(
        &mut self,
        files: &[&str],
    ) -> Result<Vec<crate::batch::FileEnrichmentResult>, LspError> {
        // Distribute work across instances using work-stealing
        let chunk_size = (files.len() + self.instances.len() - 1) / self.instances.len();
        let mut all_results = Vec::new();

        for (i, chunk) in files.chunks(chunk_size).enumerate() {
            if i < self.instances.len() {
                let results = self.instances[i].enrich_files(chunk).await?;
                all_results.extend(results);
            }
        }

        Ok(all_results)
    }

    pub async fn submit_batch(&mut self, _files: &[&str]) {
        // Work-stealing implementation
    }

    pub fn get_stats(&self) -> PoolStats {
        PoolStats::default()
    }
}
```

**Commit:**
```bash
git commit -m "feat(lsp): add multi-instance LSP pool with work-stealing (I4, I5, I5a)"
```

---

### Task 34: Add LSP Configuration Validation

**Files:**
- Modify: `crates/codegraph-lsp/src/lib.rs`

**Step 1: Write the failing test**

```rust
#[test]
fn test_validate_typescript_config() {
    let config = LspConfig {
        enabled: true,
        typescript: Some(LspServerConfig {
            enabled: true,
            server: "typescript-language-server".to_string(),
            args: vec!["--stdio".to_string()],
        }),
        ..Default::default()
    };

    assert!(validate_lsp_config(&config).is_ok());
}

#[test]
fn test_validate_empty_server_fails() {
    let config = LspConfig {
        enabled: true,
        typescript: Some(LspServerConfig {
            enabled: true,
            server: "".to_string(), // Empty!
            args: vec![],
        }),
        ..Default::default()
    };

    assert!(validate_lsp_config(&config).is_err());
}
```

**Step 2-5:** Implement validation.

**Commit:**
```bash
git commit -m "feat(lsp): add configuration validation"
```

---

## Milestone 6: Incremental Updates (Tasks 35-42)

### Task 35: Add enrichment_deps Table Operations (I2)

**Files:**
- Create: `crates/codegraph-db/src/enrichment_deps.rs`

**Step 1: Write the failing test**

```rust
#[test]
fn test_insert_enrichment_dep() {
    let db = DatabaseConnection::open_in_memory().unwrap();
    run_migrations(db.conn()).unwrap();

    insert_enrichment_dep(db.conn(), "node_123", "src/utils.ts").unwrap();

    let deps = get_deps_for_node(db.conn(), "node_123").unwrap();
    assert_eq!(deps.len(), 1);
    assert_eq!(deps[0], "src/utils.ts");
}

#[test]
fn test_get_affected_nodes_by_file() {
    let db = DatabaseConnection::open_in_memory().unwrap();
    run_migrations(db.conn()).unwrap();

    insert_enrichment_dep(db.conn(), "node_1", "src/types.ts").unwrap();
    insert_enrichment_dep(db.conn(), "node_2", "src/types.ts").unwrap();
    insert_enrichment_dep(db.conn(), "node_3", "src/other.ts").unwrap();

    let affected = get_nodes_depending_on_file(db.conn(), "src/types.ts").unwrap();
    assert_eq!(affected.len(), 2);
    assert!(affected.contains(&"node_1".to_string()));
    assert!(affected.contains(&"node_2".to_string()));
}

#[test]
fn test_clear_deps_for_node() {
    let db = DatabaseConnection::open_in_memory().unwrap();
    run_migrations(db.conn()).unwrap();

    insert_enrichment_dep(db.conn(), "node_1", "a.ts").unwrap();
    insert_enrichment_dep(db.conn(), "node_1", "b.ts").unwrap();

    clear_deps_for_node(db.conn(), "node_1").unwrap();

    let deps = get_deps_for_node(db.conn(), "node_1").unwrap();
    assert!(deps.is_empty());
}
```

**Step 2-5:** Implement enrichment_deps operations.

```rust
// crates/codegraph-db/src/enrichment_deps.rs
use rusqlite::{Connection, params};
use crate::error::DbError;

pub fn insert_enrichment_dep(conn: &Connection, node_id: &str, depends_on_file: &str) -> Result<(), DbError> {
    conn.execute(
        "INSERT OR IGNORE INTO enrichment_deps (node_id, depends_on_file) VALUES (?1, ?2)",
        params![node_id, depends_on_file],
    )?;
    Ok(())
}

pub fn get_deps_for_node(conn: &Connection, node_id: &str) -> Result<Vec<String>, DbError> {
    let mut stmt = conn.prepare("SELECT depends_on_file FROM enrichment_deps WHERE node_id = ?1")?;
    let deps = stmt.query_map(params![node_id], |row| row.get(0))?
        .collect::<Result<Vec<String>, _>>()?;
    Ok(deps)
}

pub fn get_nodes_depending_on_file(conn: &Connection, file_path: &str) -> Result<Vec<String>, DbError> {
    let mut stmt = conn.prepare("SELECT node_id FROM enrichment_deps WHERE depends_on_file = ?1")?;
    let nodes = stmt.query_map(params![file_path], |row| row.get(0))?
        .collect::<Result<Vec<String>, _>>()?;
    Ok(nodes)
}

pub fn clear_deps_for_node(conn: &Connection, node_id: &str) -> Result<(), DbError> {
    conn.execute("DELETE FROM enrichment_deps WHERE node_id = ?1", params![node_id])?;
    Ok(())
}
```

**Commit:**
```bash
git commit -m "feat(db): add enrichment_deps table operations (I2)"
```

---

### Task 36: Populate enrichment_deps from LSP

**Files:**
- Modify: `crates/codegraph-lsp/src/batch.rs`

**Step 1: Write the failing test**

```rust
#[test]
fn test_lsp_definition_populates_deps() {
    // When LSP resolves an import to another file,
    // we record that dependency
    let mut enricher = MockEnricher::new();
    enricher.set_definition_response("src/api.ts", "src/types.ts");

    let result = enricher.enrich_node_sync(
        "node_123",
        "src/api.ts",
        10, 5, // import position
    ).unwrap();

    assert_eq!(result.resolved_import_path, Some("src/types.ts".to_string()));
    assert!(result.deps_to_record.contains(&"src/types.ts".to_string()));
}
```

**Commit:**
```bash
git commit -m "feat(lsp): populate enrichment_deps from LSP definition responses"
```

---

### Task 37: Implement Selective Scope (L4a)

**Files:**
- Create: `crates/codegraph-sync/src/selective.rs`

**Step 1: Write the failing test**

```rust
#[test]
fn test_selective_scope_includes_changed_files() {
    let changed_files = vec!["src/api.ts", "src/utils.ts"];
    let scope = SelectiveScope::from_changed_files(&changed_files);

    assert!(scope.should_enrich("src/api.ts"));
    assert!(scope.should_enrich("src/utils.ts"));
    assert!(!scope.should_enrich("src/other.ts"));
}

#[test]
fn test_selective_scope_includes_dependents() {
    let db = setup_test_db_with_deps();
    // node_1 depends on types.ts
    // node_2 depends on types.ts

    let changed_files = vec!["src/types.ts"];
    let scope = SelectiveScope::from_changed_files_with_deps(&db, &changed_files);

    // Should include nodes that depend on changed files
    assert!(scope.should_enrich_node("node_1"));
    assert!(scope.should_enrich_node("node_2"));
}

#[test]
fn test_selective_scope_includes_new_imports() {
    // L4a: selective scope includes new/changed imports
    let old_imports = vec!["./utils"];
    let new_imports = vec!["./utils", "./newModule"];

    let scope = SelectiveScope::from_import_diff(&old_imports, &new_imports);

    // Nodes with new imports should be re-enriched
    assert!(scope.has_new_imports());
}
```

**Commit:**
```bash
git commit -m "feat(sync): implement selective enrichment scope (L4a)"
```

---

### Task 38: Add Edge Change Detection (I11)

**Files:**
- Create: `crates/codegraph-sync/src/edge_diff.rs`

**Step 1: Write the failing test**

```rust
#[test]
fn test_edge_snapshot_and_diff() {
    let db = setup_test_db();

    // Take snapshot before extraction
    let before = EdgeSnapshot::capture(&db);

    // Simulate extraction adding new edges
    add_edge(&db, "node_a", "node_b", EdgeKind::Calls);

    // Take snapshot after
    let after = EdgeSnapshot::capture(&db);

    // Diff should show affected nodes
    let diff = EdgeDiff::compute(&before, &after);

    assert!(diff.affected_nodes.contains(&"node_a".to_string()));
    assert!(diff.affected_nodes.contains(&"node_b".to_string()));
}

#[test]
fn test_edge_removal_detected() {
    let db = setup_test_db();
    add_edge(&db, "node_a", "node_b", EdgeKind::Calls);

    let before = EdgeSnapshot::capture(&db);

    remove_edge(&db, "node_a", "node_b");

    let after = EdgeSnapshot::capture(&db);
    let diff = EdgeDiff::compute(&before, &after);

    // Removed edge should mark both nodes as affected
    assert!(diff.affected_nodes.contains(&"node_a".to_string()));
}
```

**Commit:**
```bash
git commit -m "feat(sync): add edge change detection via snapshot/diff (I11)"
```

---

### Task 39: Add File Locking (I12)

**Files:**
- Create: `crates/codegraph-sync/src/lock.rs`

**Step 1: Write the failing test**

```rust
#[test]
fn test_acquire_index_lock() {
    let temp = TempDir::new().unwrap();
    let codegraph_dir = temp.path().join(".codegraph");
    fs::create_dir_all(&codegraph_dir).unwrap();

    let lock = IndexLock::acquire(&codegraph_dir).unwrap();
    assert!(lock.is_held());

    // Lock file should exist
    assert!(codegraph_dir.join("index.lock").exists());
}

#[test]
fn test_lock_prevents_concurrent_access() {
    let temp = TempDir::new().unwrap();
    let codegraph_dir = temp.path().join(".codegraph");
    fs::create_dir_all(&codegraph_dir).unwrap();

    let lock1 = IndexLock::acquire(&codegraph_dir).unwrap();

    // Second acquire should fail
    let lock2 = IndexLock::try_acquire(&codegraph_dir);
    assert!(lock2.is_err());

    drop(lock1);

    // Now it should succeed
    let lock3 = IndexLock::acquire(&codegraph_dir).unwrap();
    assert!(lock3.is_held());
}

#[test]
fn test_stale_lock_detected_by_mtime() {
    let temp = TempDir::new().unwrap();
    let codegraph_dir = temp.path().join(".codegraph");
    fs::create_dir_all(&codegraph_dir).unwrap();

    // Create stale lock file (old mtime)
    let lock_path = codegraph_dir.join("index.lock");
    fs::write(&lock_path, "stale").unwrap();

    // Set mtime to 10 minutes ago
    let old_time = std::time::SystemTime::now() - std::time::Duration::from_secs(600);
    filetime::set_file_mtime(&lock_path, filetime::FileTime::from_system_time(old_time)).unwrap();

    // Should be able to acquire (stale lock broken)
    let lock = IndexLock::acquire(&codegraph_dir).unwrap();
    assert!(lock.is_held());
}
```

**Commit:**
```bash
git commit -m "feat(sync): add file locking with stale detection (I12)"
```

---

### Task 40: Implement Full Re-embed Triggers (I13)

**Files:**
- Modify: `crates/codegraph-core/src/embedding.rs`

**Step 1: Write the failing test**

```rust
#[test]
fn test_schema_migration_triggers_reembed() {
    let db = setup_test_db();
    set_metadata(&db, "schema_version", "1");

    // Migrate schema
    run_migrations(db.conn()).unwrap(); // Now v2

    assert!(should_full_reembed(&db));
}

#[test]
fn test_config_change_triggers_reembed() {
    let db = setup_test_db();
    let old_config = EmbeddingTextConfig { max_tokens: 2000, ..Default::default() };
    save_config_hash(&db, &old_config);

    let new_config = EmbeddingTextConfig { max_tokens: 3000, ..Default::default() };

    assert!(config_changed(&db, &new_config));
    assert!(should_full_reembed(&db));
}

#[test]
fn test_force_flag_triggers_reembed() {
    let options = IndexOptions { force_reembed: true, ..Default::default() };
    assert!(should_full_reembed_with_options(&options));
}

#[test]
fn test_model_change_triggers_reembed() {
    let db = setup_test_db();
    set_metadata(&db, "embedding_model_hash", "abc123");

    // Model changed
    let new_model_hash = "def456";
    assert!(model_changed(&db, new_model_hash));
}
```

**Commit:**
```bash
git commit -m "feat(core): implement full re-embed triggers (I13)"
```

---

### Task 41: Add Cascade Depth Config (I3)

**Step 1: Write the failing test**

```rust
#[test]
fn test_cascade_depth_default_one() {
    let config = EnrichmentConfig::default();
    assert_eq!(config.cascade_depth, 1);
}

#[test]
fn test_cascade_finds_direct_dependents_only() {
    let db = setup_test_db_with_deps();
    // a.ts -> b.ts -> c.ts (chain)

    let config = EnrichmentConfig { cascade_depth: 1, ..Default::default() };
    let affected = find_affected_nodes(&db, &["a.ts"], &config);

    // Should only find direct dependents of a.ts
    assert!(affected.contains(&"b.ts"));
    assert!(!affected.contains(&"c.ts")); // Not at depth 1
}

#[test]
fn test_cascade_depth_two_finds_transitive() {
    let db = setup_test_db_with_deps();

    let config = EnrichmentConfig { cascade_depth: 2, ..Default::default() };
    let affected = find_affected_nodes(&db, &["a.ts"], &config);

    // Depth 2 should find transitive
    assert!(affected.contains(&"b.ts"));
    assert!(affected.contains(&"c.ts"));
}
```

**Commit:**
```bash
git commit -m "feat(sync): add configurable cascade depth (I3)"
```

---

### Task 42: Integrate with Sync Command

**Step 1: Write the failing test**

```rust
#[test]
fn test_sync_uses_selective_scope_by_default() {
    let project = setup_test_project();
    index_project(&project).unwrap();

    // Modify one file
    modify_file(&project, "src/api.ts");

    // Sync should only re-enrich affected nodes
    let stats = sync_project(&project).unwrap();

    assert!(stats.nodes_enriched < stats.total_nodes);
    assert!(stats.nodes_enriched > 0);
}
```

**Commit:**
```bash
git commit -m "feat(sync): integrate incremental enrichment with sync command"
```

---

## Milestone 7: New Extraction (Tasks 43-48)

### Task 43: Extract Code Snippets (E4)

**Files:**
- Modify: `crates/codegraph-extraction/src/extractor.rs`

**Step 1: Write the failing test**

```rust
#[test]
fn test_extract_code_snippet_under_limit() {
    let source = r#"
function add(a: number, b: number): number {
    return a + b;
}
"#;

    let node = extract_function(source, "add");
    let snippet = extract_code_snippet(source, &node, 50); // 50 line limit

    assert!(snippet.is_some());
    assert!(snippet.unwrap().contains("return a + b"));
}

#[test]
fn test_extract_code_snippet_truncates_long_functions() {
    let source = generate_long_function(100); // 100 lines

    let node = extract_function(&source, "longFunc");
    let snippet = extract_code_snippet(&source, &node, 50);

    assert!(snippet.is_some());
    let snippet = snippet.unwrap();
    let line_count = snippet.lines().count();
    assert!(line_count <= 50);
    assert!(snippet.ends_with("// ... truncated"));
}
```

**Commit:**
```bash
git commit -m "feat(extraction): extract code snippets with configurable limit (E4)"
```

---

### Task 44: Extract Thrown Errors (E8)

**Files:**
- Modify: `crates/codegraph-extraction/src/extractor.rs`

**Step 1: Write the failing test**

```rust
#[test]
fn test_extract_thrown_errors_typescript() {
    let source = r#"
function validate(input: string): void {
    if (!input) {
        throw new ValidationError("Input required");
    }
    if (input.length > 100) {
        throw new LengthError("Too long");
    }
}
"#;

    let node = extract_function(source, "validate");
    let errors = extract_thrown_errors(source, &node, Language::TypeScript);

    assert_eq!(errors.len(), 2);
    assert!(errors.contains(&"ValidationError".to_string()));
    assert!(errors.contains(&"LengthError".to_string()));
}

#[test]
fn test_extract_thrown_errors_rust() {
    let source = r#"
fn validate(input: &str) -> Result<(), AppError> {
    if input.is_empty() {
        return Err(AppError::ValidationError("Input required".into()));
    }
    Ok(())
}
"#;

    let node = extract_function(source, "validate");
    let errors = extract_thrown_errors(source, &node, Language::Rust);

    // Rust uses Result types, extract from Err() calls
    assert!(errors.contains(&"AppError::ValidationError".to_string()));
}
```

**Commit:**
```bash
git commit -m "feat(extraction): extract thrown error types (E8)"
```

---

### Task 45: Implement Test Convention Detection (E6a)

**Files:**
- Create: `crates/codegraph-extraction/src/test_detection.rs`

**Step 1: Write the failing test**

```rust
#[test]
fn test_detect_test_file_typescript() {
    assert!(is_test_file("src/utils.test.ts", Language::TypeScript));
    assert!(is_test_file("src/utils.spec.ts", Language::TypeScript));
    assert!(is_test_file("__tests__/utils.ts", Language::TypeScript));
    assert!(!is_test_file("src/utils.ts", Language::TypeScript));
}

#[test]
fn test_detect_test_file_rust() {
    assert!(is_test_file("src/utils_test.rs", Language::Rust));
    assert!(is_test_file("tests/integration.rs", Language::Rust));
    assert!(!is_test_file("src/utils.rs", Language::Rust));
}

#[test]
fn test_detect_test_file_python() {
    assert!(is_test_file("test_utils.py", Language::Python));
    assert!(is_test_file("tests/test_api.py", Language::Python));
    assert!(!is_test_file("utils.py", Language::Python));
}

#[test]
fn test_find_test_functions_for_symbol() {
    let test_source = r#"
describe('UserService', () => {
    it('should create user', () => {});
    it('should delete user', () => {});
});
"#;

    let tests = find_test_names_for_symbol(test_source, "UserService", Language::TypeScript);
    assert_eq!(tests.len(), 2);
    assert!(tests.contains(&"should create user".to_string()));
}
```

**Commit:**
```bash
git commit -m "feat(extraction): implement test file convention detection (E6a)"
```

---

### Task 46: Add Test Association via Imports (E6)

**Files:**
- Modify: `crates/codegraph-extraction/src/test_detection.rs`

**Step 1: Write the failing test**

```rust
#[test]
fn test_associate_via_import() {
    let test_source = r#"
import { UserService } from '../services/user';

describe('UserService', () => {
    it('should work', () => {});
});
"#;

    let imports = extract_imports(test_source, Language::TypeScript);
    let associations = associate_tests_via_imports(&imports, "services/user.ts");

    assert!(associations.contains(&"should work".to_string()));
}

#[test]
fn test_hybrid_association_prefers_convention() {
    // E6: Convention first, imports as fallback
    let symbol_name = "UserService";
    let test_file = "user.test.ts";

    // Convention matches - use convention
    let result = find_tests_for_symbol(symbol_name, test_file, Language::TypeScript);
    assert!(result.method == AssociationMethod::Convention);
}
```

**Commit:**
```bash
git commit -m "feat(extraction): add test association via imports (E6)"
```

---

### Task 47: Extract Package Names (M1)

**Files:**
- Create: `crates/codegraph-extraction/src/package.rs`

**Step 1: Write the failing test**

```rust
#[test]
fn test_extract_package_from_package_json() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("package.json"), r#"{"name": "@myorg/utils"}"#).unwrap();

    let package = extract_package_name(temp.path(), "src/index.ts").unwrap();
    assert_eq!(package, "@myorg/utils");
}

#[test]
fn test_extract_package_from_cargo_toml() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("Cargo.toml"), r#"
[package]
name = "my-crate"
"#).unwrap();

    let package = extract_package_name(temp.path(), "src/lib.rs").unwrap();
    assert_eq!(package, "my-crate");
}

#[test]
fn test_extract_package_from_pubspec_yaml() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("pubspec.yaml"), "name: my_app\n").unwrap();

    let package = extract_package_name(temp.path(), "lib/main.dart").unwrap();
    assert_eq!(package, "my_app");
}

#[test]
fn test_fallback_to_directory_name() {
    // M2: No manifest - use directory path
    let temp = TempDir::new().unwrap();
    // No package.json, Cargo.toml, etc.

    let package = extract_package_name(temp.path(), "src/index.ts");
    // Should use directory name as fallback
    assert!(package.is_some());
}
```

**Commit:**
```bash
git commit -m "feat(extraction): extract package names from manifests (M1, M2)"
```

---

### Task 48: Add Workspace Detection (M3)

**Files:**
- Modify: `crates/codegraph-extraction/src/package.rs`

**Step 1: Write the failing test**

```rust
#[test]
fn test_monorepo_preserves_package_name() {
    let temp = TempDir::new().unwrap();

    // Root workspace
    fs::write(temp.path().join("package.json"), r#"
{
    "name": "monorepo-root",
    "workspaces": ["packages/*"]
}
"#).unwrap();

    // Sub-package
    fs::create_dir_all(temp.path().join("packages/utils")).unwrap();
    fs::write(temp.path().join("packages/utils/package.json"), r#"
{
    "name": "@myorg/utils"
}
"#).unwrap();

    let package = extract_package_name(
        temp.path(),
        "packages/utils/src/index.ts"
    ).unwrap();

    // M3: Should preserve sub-package name, not root
    assert_eq!(package, "@myorg/utils");
}

#[test]
fn test_cargo_workspace_detection() {
    let temp = TempDir::new().unwrap();

    // Root workspace
    fs::write(temp.path().join("Cargo.toml"), r#"
[workspace]
members = ["crates/*"]
"#).unwrap();

    // Sub-crate
    fs::create_dir_all(temp.path().join("crates/utils")).unwrap();
    fs::write(temp.path().join("crates/utils/Cargo.toml"), r#"
[package]
name = "my-utils"
"#).unwrap();

    let package = extract_package_name(
        temp.path(),
        "crates/utils/src/lib.rs"
    ).unwrap();

    assert_eq!(package, "my-utils");
}
```

**Commit:**
```bash
git commit -m "feat(extraction): add workspace/monorepo detection (M3)"
```

---

## Milestone 8: Quality & Polish (Tasks 49-54)

### Task 49: Create Evaluation Test Cases (Q1)

**Files:**
- Create: `crates/codegraph-core/src/eval.rs`

**Step 1: Write the failing test**

```rust
#[test]
fn test_golden_query_format() {
    let query = GoldenQuery {
        query: "payment processing function".to_string(),
        expected_top_results: vec![
            "src/payment.ts::processPayment".to_string(),
            "src/payment.ts::chargeCard".to_string(),
        ],
        tags: vec!["payment".to_string()],
    };

    assert!(query.is_valid());
}

#[test]
fn test_evaluate_single_query() {
    let db = setup_indexed_test_project();
    let query = GoldenQuery {
        query: "user authentication".to_string(),
        expected_top_results: vec!["src/auth.ts::authenticate".to_string()],
        tags: vec![],
    };

    let result = evaluate_query(&db, &query, 10).unwrap();

    assert!(result.recall_at_10 >= 0.0);
    assert!(result.recall_at_10 <= 1.0);
    assert!(result.mrr >= 0.0);
}

#[test]
fn test_evaluate_query_set() {
    let db = setup_indexed_test_project();
    let queries = load_golden_queries("test_fixtures/golden_queries.json").unwrap();

    let results = evaluate_query_set(&db, &queries).unwrap();

    assert!(results.avg_recall_at_10 >= 0.0);
    assert!(results.avg_mrr >= 0.0);
}
```

**Commit:**
```bash
git commit -m "feat(eval): create evaluation test cases with golden queries (Q1)"
```

---

### Task 50: Implement A/B Comparison (Q1)

**Files:**
- Modify: `crates/codegraph-core/src/eval.rs`

**Step 1: Write the failing test**

```rust
#[test]
fn test_ab_comparison() {
    let baseline = EvalResults {
        avg_recall_at_10: 0.7,
        avg_mrr: 0.5,
        query_count: 100,
    };

    let current = EvalResults {
        avg_recall_at_10: 0.75,
        avg_mrr: 0.55,
        query_count: 100,
    };

    let comparison = ABComparison::compare(&baseline, &current);

    assert!(comparison.recall_delta > 0.0);
    assert!(comparison.mrr_delta > 0.0);
    assert!(comparison.is_improvement());
}

#[test]
fn test_ab_detects_regression() {
    let baseline = EvalResults {
        avg_recall_at_10: 0.8,
        avg_mrr: 0.6,
        query_count: 100,
    };

    let current = EvalResults {
        avg_recall_at_10: 0.7, // Worse!
        avg_mrr: 0.5,
        query_count: 100,
    };

    let comparison = ABComparison::compare(&baseline, &current);

    assert!(comparison.recall_delta < 0.0);
    assert!(comparison.is_regression());
}

#[test]
fn test_save_and_load_baseline() {
    let temp = TempDir::new().unwrap();
    let results = EvalResults {
        avg_recall_at_10: 0.75,
        avg_mrr: 0.55,
        query_count: 50,
    };

    save_baseline(&temp.path().join("baseline.json"), &results).unwrap();
    let loaded = load_baseline(&temp.path().join("baseline.json")).unwrap();

    assert_eq!(loaded.avg_recall_at_10, results.avg_recall_at_10);
}
```

**Commit:**
```bash
git commit -m "feat(eval): implement A/B comparison for quality measurement (Q1)"
```

---

### Task 51: Add CLI Flags

**Files:**
- Modify: `crates/codegraph-cli/src/main.rs`

**Step 1: Write the failing test**

```rust
#[test]
fn test_cli_force_reembed_flag() {
    let args = Args::parse_from(&["codegraph", "index", "--force-reembed", "."]);
    assert!(args.force_reembed);
}

#[test]
fn test_cli_eval_save_baseline() {
    let args = Args::parse_from(&["codegraph", "eval", "--save-baseline", "."]);
    assert!(args.save_baseline);
}

#[test]
fn test_cli_eval_compare() {
    let args = Args::parse_from(&["codegraph", "eval", "--compare", "baseline.json", "."]);
    assert_eq!(args.compare, Some("baseline.json".to_string()));
}
```

**Commit:**
```bash
git commit -m "feat(cli): add --force-reembed and eval flags"
```

---

### Task 52: Add Git Activity Boost (B8)

**Files:**
- Create: `crates/codegraph-vectors/src/git_boost.rs`

**Step 1: Write the failing test**

```rust
#[test]
fn test_git_activity_boost_prioritizes_recent() {
    let files = vec![
        FileActivity { path: "old.ts", last_modified: days_ago(30), churn: 10 },
        FileActivity { path: "recent.ts", last_modified: days_ago(1), churn: 5 },
    ];

    let sorted = sort_by_activity(&files);

    // Recent file should come first despite lower churn
    assert_eq!(sorted[0].path, "recent.ts");
}

#[test]
fn test_truncation_prefers_active_content() {
    let config = EmbeddingTextConfig {
        max_tokens: 100,
        git_activity_boost: true,
        ..Default::default()
    };

    let context = GraphContext {
        callees: vec!["activeFunc".to_string(), "staleFunc".to_string()],
        ..Default::default()
    };

    let activity = HashMap::from([
        ("activeFunc", days_ago(1)),
        ("staleFunc", days_ago(100)),
    ]);

    let text = build_text_with_activity(&config, &context, &activity);

    // When truncating, should keep activeFunc
    assert!(text.contains("activeFunc"));
}
```

**Commit:**
```bash
git commit -m "feat(vectors): add optional git activity boost for truncation (B8)"
```

---

### Task 53: Final Integration Test

**Files:**
- Create: `tests/integration/enrichment_e2e.rs`

**Step 1: Write the test**

```rust
#[test]
#[ignore] // Requires full setup
fn test_full_enrichment_pipeline() {
    let temp = setup_full_test_project();

    // Index with enrichment
    let result = index_project_with_enrichment(&temp, EnrichmentConfig::default());
    assert!(result.is_ok());

    // Verify embeddings include enriched content
    let node = get_node(&temp, "src/api.ts::handleRequest");
    assert!(node.inferred_type.is_some());
    assert!(!node.thrown_errors.is_empty());

    // Verify search quality improved
    let results = search(&temp, "error handling in API");
    assert!(results[0].qualified_name.contains("handleRequest"));
}

#[test]
#[ignore]
fn test_incremental_enrichment_after_change() {
    let temp = setup_full_test_project();
    index_project_with_enrichment(&temp, EnrichmentConfig::default()).unwrap();

    let stats_before = get_stats(&temp);

    // Modify one file
    modify_file(&temp, "src/utils.ts");

    // Sync should only re-enrich affected nodes
    let sync_result = sync_project(&temp).unwrap();
    assert!(sync_result.nodes_enriched < stats_before.total_nodes);
}
```

**Commit:**
```bash
git commit -m "test: add end-to-end enrichment integration tests"
```

---

### Task 54: Update Documentation

**Files:**
- Modify: `README.md`
- Modify: `CLAUDE.md`

**Step 1:** Add enrichment documentation.

**Commit:**
```bash
git commit -m "docs: add embedding enrichment documentation"
```

---

## Summary

**Total: 54 tasks** across **8 milestones**

Changes from v1 plan:
- Added Task 2: Update Node struct (Critical fix)
- Added Task 3: Update QueryBuilder (Critical fix)
- Added Task 21: Create codegraph-lsp crate (Critical fix)
- Added Task 30: Async/Sync bridge (Critical fix)
- Fixed test setup helpers throughout
- Fixed JSON comparison in SQL queries
- Corrected tiktoken-rs usage
- Proper dependency ordering

---

## Decision Traceability

| Design Decision | Tasks |
|-----------------|-------|
| A1-A2 (Architecture) | 20, 42 |
| L1-L5 (LSP) | 21-30 |
| I1-I13 (Incremental) | 35-42 |
| G1-G6 (Graph) | 15-19 |
| E1-E8 (Embedding Content) | 7-14, 43-46 |
| M1-M3 (Module/Package) | 12, 47-48 |
| B1-B8 (Budget) | 8-9, 14, 19, 52 |
| C1 (Config) | 5 |
| Q1 (Quality) | 50 |

---

Plan v2 complete. Ready for implementation.
