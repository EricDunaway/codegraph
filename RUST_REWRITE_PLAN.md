# CodeGraph Rust Rewrite Plan

## Executive Summary

This document outlines a comprehensive plan to rewrite CodeGraph from TypeScript/Node.js to Rust. The goal is to create a high-performance, memory-safe, local-first code intelligence system with strict guarantees that **no data leaves the local machine**.

---

## Table of Contents

1. [Architecture Overview](#1-architecture-overview)
2. [Crate Structure](#2-crate-structure)
3. [Module Implementation Plan](#3-module-implementation-plan)
4. [Dependency Mapping](#4-dependency-mapping)
5. [Data Flow & Storage](#5-data-flow--storage)
6. [Security Design](#6-security-design)
7. [Migration Strategy](#7-migration-strategy)
8. [Testing Strategy](#8-testing-strategy)
9. [Adversarial Scrutiny](#9-adversarial-scrutiny)
10. [Security Audit](#10-security-audit)
11. [Clarifying Questions](#11-clarifying-questions)

---

## 1. Architecture Overview

### High-Level Design

```
┌─────────────────────────────────────────────────────────────────┐
│                         codegraph-cli                            │
│                    (Binary: user interface)                      │
└─────────────────────────────────┬───────────────────────────────┘
                                  │
┌─────────────────────────────────▼───────────────────────────────┐
│                        codegraph-core                            │
│                    (Library: main API)                           │
├─────────────────────────────────────────────────────────────────┤
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌──────────────────┐ │
│  │extraction│  │resolution│  │  graph   │  │     vectors      │ │
│  │(treesit) │  │(imports) │  │(traverse)│  │  (embeddings)    │ │
│  └────┬─────┘  └────┬─────┘  └────┬─────┘  └────────┬─────────┘ │
│       │             │             │                  │           │
│  ┌────▼─────────────▼─────────────▼──────────────────▼─────────┐│
│  │                       codegraph-db                          ││
│  │                 (SQLite + rusqlite)                         ││
│  └─────────────────────────────────────────────────────────────┘│
└─────────────────────────────────────────────────────────────────┘
                                  │
┌─────────────────────────────────▼───────────────────────────────┐
│                        codegraph-mcp                             │
│                  (MCP Server: stdio transport)                   │
└─────────────────────────────────────────────────────────────────┘
```

### Design Principles

1. **Local-First**: All data stored locally in `.codegraph/` directory
2. **No Network by Default**: Network capabilities disabled unless explicitly enabled
3. **Memory Safety**: Leverage Rust's ownership model
4. **Zero-Copy Where Possible**: Minimize allocations during parsing
5. **Compile-Time Guarantees**: Use Rust's type system for correctness
6. **Deterministic**: Same input always produces same output

---

## 2. Crate Structure

### Workspace Layout

```
codegraph-rs/
├── Cargo.toml                    # Workspace definition
├── crates/
│   ├── codegraph/                # Main library (public API)
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs            # CodeGraph struct, public API
│   │       ├── config.rs         # Configuration types and loading
│   │       ├── types.rs          # Node, Edge, enums
│   │       ├── errors.rs         # thiserror-based error types
│   │       └── utils.rs          # Mutex, batching utilities
│   │
│   ├── codegraph-db/             # Database layer
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── connection.rs     # DatabaseConnection
│   │       ├── queries.rs        # QueryBuilder (prepared statements)
│   │       ├── schema.rs         # Embedded schema.sql
│   │       └── migrations.rs     # Schema migrations
│   │
│   ├── codegraph-extraction/     # Tree-sitter parsing
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── orchestrator.rs   # ExtractionOrchestrator
│   │       ├── parser.rs         # Tree-sitter wrapper
│   │       ├── languages/        # Per-language extractors
│   │       │   ├── mod.rs
│   │       │   ├── typescript.rs # + JS/TSX/JSX, embedded GraphQL
│   │       │   ├── rust.rs       # + attribute macros
│   │       │   ├── php.rs
│   │       │   ├── dart.rs       # + Flutter patterns, embedded GraphQL
│   │       │   ├── graphql.rs    # Standalone GraphQL
│   │       │   ├── bash.rs       # Shell scripts
│   │       │   ├── hcl.rs        # Terraform, Vault, Nomad
│   │       │   ├── python.rs     # Secondary
│   │       │   ├── go.rs         # Secondary
│   │       │   └── ... (8 more secondary)
│   │       ├── embedded.rs       # Embedded language extraction
│   │       └── grammars.rs       # Grammar loading
│   │
│   ├── codegraph-resolution/     # Reference resolution
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── resolver.rs       # ReferenceResolver
│   │       ├── imports.rs        # Import path resolution
│   │       ├── names.rs          # Name matching
│   │       └── frameworks/       # Framework-specific patterns
│   │           ├── mod.rs
│   │           ├── react.rs      # React/Next.js
│   │           ├── express.rs    # Express/Node
│   │           ├── flutter.rs    # Flutter widgets, routes
│   │           ├── riverpod.rs   # Riverpod state management
│   │           ├── bloc.rs       # Bloc/Cubit patterns
│   │           ├── getit.rs      # get_it dependency injection
│   │           ├── terraform.rs  # Terraform resources, modules
│   │           └── ... (rust, go, python, etc.)
│   │
│   ├── codegraph-graph/          # Graph algorithms
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── traversal.rs      # BFS/DFS, GraphTraverser
│   │       └── queries.rs        # High-level graph queries
│   │
│   ├── codegraph-vectors/        # Embeddings (optional feature)
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── embedder.rs       # ONNX inference
│   │       ├── manager.rs        # VectorManager
│   │       └── search.rs         # Similarity search
│   │
│   ├── codegraph-context/        # Context building
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── builder.rs        # ContextBuilder
│   │       └── formatter.rs      # Markdown/JSON output
│   │
│   ├── codegraph-mcp/            # MCP server
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── server.rs         # MCPServer
│   │       ├── tools.rs          # Tool definitions
│   │       └── transport.rs      # Stdio transport
│   │
│   ├── codegraph-sync/           # Incremental sync
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       └── git_hooks.rs      # Git hook management
│   │
│   └── codegraph-cli/            # CLI binary
│       ├── Cargo.toml
│       └── src/
│           └── main.rs           # CLI entry point
│
├── tests/                        # Integration tests
│   ├── extraction_tests.rs
│   ├── graph_tests.rs
│   └── ...
│
└── benches/                      # Benchmarks
    └── indexing_bench.rs
```

### Feature Flags

```toml
# codegraph/Cargo.toml
[features]
default = ["cli"]
cli = ["clap", "indicatif"]
vectors = ["ort", "tokenizers", "sqlite-vec", "ndarray"]  # Base: embeddings + vector search (direct ort, not rust-bert)
vectors-coreml = ["vectors"]                               # macOS: GPU/Neural Engine via CoreML (auto-enabled on macOS)
dual-embeddings = ["vectors"]                              # StarEncoder (code) + nomic (text)
mcp = []                                                   # MCP server (sync stdio, no async runtime needed)
full = ["cli", "vectors", "mcp"]

# NOTE: We use direct `ort` instead of `rust-bert` because:
# 1. Direct CoreML access with ComputeUnits::CPUAndNeuralEngine for M4 Neural Engine
# 2. Smaller binary (no libtorch overhead)
# 3. Full control over ONNX session configuration
# 4. Embedding models don't need rust-bert's high-level NLP pipelines
```

**Build Configuration:**

```toml
# Cargo.toml - platform-specific dependencies
[target.'cfg(target_os = "macos")'.dependencies]
ort = { version = "2.0", features = ["load-dynamic", "coreml"] }

[target.'cfg(not(target_os = "macos"))'.dependencies]
ort = { version = "2.0", features = ["load-dynamic"] }
```

---

## 3. Module Implementation Plan

### 3.1 Core Types (`codegraph/src/types.rs`)

```rust
use serde::{Deserialize, Serialize};

/// Unique identifier for a node (32-char hex hash)
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeId(pub String);

/// Kind of code symbol
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    File,
    Module,
    Class,
    Struct,
    Interface,
    Trait,
    Protocol,
    Function,
    Method,
    Property,
    Field,
    Variable,
    Constant,
    Enum,
    EnumMember,
    TypeAlias,
    Namespace,
    Parameter,
    Import,
    Export,
    Route,
    Component,
}

/// Relationship between nodes
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    Contains,
    Calls,
    Imports,
    Exports,
    Extends,
    Implements,
    References,
    TypeOf,
    Returns,
    Instantiates,
    Overrides,
    Decorates,
}

/// Programming language
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    // === Primary Targets (first-class support with framework patterns) ===
    TypeScript,
    JavaScript,
    Tsx,
    Jsx,
    Rust,
    Php,
    Dart,           // Flutter, AngularDart
    GraphQL,        // Standalone + embedded extraction
    Bash,           // Shell scripts
    Hcl,            // Terraform, Vault, Nomad, Packer

    // === Secondary (extraction only, no framework patterns) ===
    Python,
    Go,
    Java,
    C,
    Cpp,
    CSharp,
    Ruby,
    Swift,
    Kotlin,
}

/// Visibility modifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Visibility {
    Public,
    Private,
    Protected,
    Internal,
}

/// Code symbol node
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub id: NodeId,
    pub kind: NodeKind,
    pub name: String,
    pub qualified_name: String,
    pub file_path: String,
    pub language: Language,
    pub start_line: u32,
    pub end_line: u32,
    pub start_column: u32,
    pub end_column: u32,
    pub docstring: Option<String>,
    pub signature: Option<String>,
    pub visibility: Option<Visibility>,
    pub is_exported: bool,
    pub is_async: bool,
    pub is_static: bool,
    pub is_abstract: bool,
    pub decorators: Vec<String>,
    pub type_parameters: Vec<String>,
    pub updated_at: i64,
}

/// Relationship edge
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    pub id: i64,
    pub source: NodeId,
    pub target: NodeId,
    pub kind: EdgeKind,
    pub metadata: Option<serde_json::Value>,
    pub line: Option<u32>,
    pub col: Option<u32>,
}

/// File tracking record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackedFile {
    pub path: String,
    pub content_hash: String,
    pub language: Language,
    pub size: u64,
    pub modified_at: i64,
    pub indexed_at: i64,
    pub node_count: u32,
    pub errors: Vec<String>,
}
```

### 3.2 Database Layer (`codegraph-db/`)

**Key Implementation Details:**

```rust
use rusqlite::{ffi::sqlite3_auto_extension, Connection, OpenFlags, params};
use sqlite_vec::sqlite3_vec_init;
use std::path::Path;
use std::sync::Once;

static SQLITE_VEC_INIT: Once = Once::new();

pub struct DatabaseConnection {
    conn: Connection,
    queries: QueryBuilder,
}

impl DatabaseConnection {
    pub fn open(path: &Path) -> Result<Self, DbError> {
        // Initialize sqlite-vec extension (once per process)
        SQLITE_VEC_INIT.call_once(|| {
            unsafe {
                sqlite3_auto_extension(Some(std::mem::transmute(
                    sqlite3_vec_init as *const ()
                )));
            }
        });

        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX;

        let conn = Connection::open_with_flags(path, flags)?;

        // Enable WAL mode for concurrent reads
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;

        // Apply schema
        conn.execute_batch(include_str!("schema.sql"))?;

        // Verify sqlite-vec is loaded
        let (vec_version,): (String,) = conn.query_row(
            "SELECT vec_version()",
            [],
            |row| Ok((row.get(0)?,))
        )?;
        log::debug!("sqlite-vec version: {}", vec_version);

        Ok(Self {
            queries: QueryBuilder::new(&conn)?,
            conn,
        })
    }
}

/// Pre-compiled prepared statements
pub struct QueryBuilder {
    insert_node: Statement,
    insert_edge: Statement,
    get_node_by_id: Statement,
    get_edges_by_source: Statement,
    // ... 20+ more statements
}
```

**Schema (embedded via `include_str!`):**

```sql
-- schema.sql (same as TypeScript version)
CREATE TABLE IF NOT EXISTS nodes (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    name TEXT NOT NULL,
    qualified_name TEXT NOT NULL,
    file_path TEXT NOT NULL,
    language TEXT NOT NULL,
    start_line INTEGER NOT NULL,
    end_line INTEGER NOT NULL,
    start_column INTEGER NOT NULL,
    end_column INTEGER NOT NULL,
    docstring TEXT,
    signature TEXT,
    visibility TEXT,
    is_exported INTEGER DEFAULT 0,
    is_async INTEGER DEFAULT 0,
    is_static INTEGER DEFAULT 0,
    is_abstract INTEGER DEFAULT 0,
    decorators TEXT,
    type_parameters TEXT,
    updated_at INTEGER NOT NULL
);

-- ... (full schema as in TypeScript version)

-- FTS5 for full-text search
CREATE VIRTUAL TABLE IF NOT EXISTS nodes_fts USING fts5(
    id, name, qualified_name, docstring,
    content='nodes',
    content_rowid='rowid'
);

-- sqlite-vec virtual table for vector embeddings
-- Note: Created at runtime after sqlite-vec extension is loaded
-- CREATE VIRTUAL TABLE IF NOT EXISTS vec_embeddings USING vec0(
--     node_id TEXT PRIMARY KEY,
--     embedding float[768]  -- Dimension matches embedding model
-- );
```

### 3.3 Extraction Module (`codegraph-extraction/`)

**Tree-sitter Integration:**

```rust
use tree_sitter::{Parser, Language as TsLanguage, Node as TsNode};
use std::collections::HashMap;

pub struct TreeSitterParser {
    parsers: HashMap<Language, Parser>,
}

impl TreeSitterParser {
    pub fn new() -> Self {
        Self {
            parsers: HashMap::new(),
        }
    }

    pub fn parse(&mut self, source: &str, lang: Language) -> Result<ParseResult, ParseError> {
        let parser = self.get_or_create_parser(lang)?;
        let tree = parser.parse(source, None)
            .ok_or(ParseError::ParseFailed)?;

        let mut extractor = LanguageExtractor::for_language(lang);
        extractor.extract(&tree, source)
    }

    fn get_or_create_parser(&mut self, lang: Language) -> Result<&mut Parser, ParseError> {
        if !self.parsers.contains_key(&lang) {
            let mut parser = Parser::new();
            parser.set_language(&get_grammar(lang)?)?;
            self.parsers.insert(lang, parser);
        }
        Ok(self.parsers.get_mut(&lang).unwrap())
    }
}

fn get_grammar(lang: Language) -> Result<TsLanguage, ParseError> {
    match lang {
        // Primary targets
        Language::TypeScript => Ok(tree_sitter_typescript::language_typescript()),
        Language::Tsx => Ok(tree_sitter_typescript::language_tsx()),
        Language::JavaScript | Language::Jsx => Ok(tree_sitter_javascript::language()),
        Language::Rust => Ok(tree_sitter_rust::language()),
        Language::Php => Ok(tree_sitter_php::language_php()),
        Language::Dart => Ok(tree_sitter_dart::language()),
        Language::GraphQL => Ok(tree_sitter_graphql::language()),
        Language::Bash => Ok(tree_sitter_bash::language()),
        Language::Hcl => Ok(tree_sitter_hcl::language()),

        // Secondary targets
        Language::Python => Ok(tree_sitter_python::language()),
        Language::Go => Ok(tree_sitter_go::language()),
        Language::Java => Ok(tree_sitter_java::language()),
        Language::C => Ok(tree_sitter_c::language()),
        Language::Cpp => Ok(tree_sitter_cpp::language()),
        Language::CSharp => Ok(tree_sitter_c_sharp::language()),
        Language::Ruby => Ok(tree_sitter_ruby::language()),
        Language::Swift => Ok(tree_sitter_swift::language()),
        Language::Kotlin => Ok(tree_sitter_kotlin::language()),
    }
}
```

**Language-Specific Extractor Trait:**

```rust
pub trait LanguageExtractor {
    /// AST node types that represent functions
    fn function_types(&self) -> &[&str];

    /// AST node types that represent classes
    fn class_types(&self) -> &[&str];

    /// AST node types that represent method definitions
    fn method_types(&self) -> &[&str];

    /// Extract function signature from AST node
    fn extract_signature(&self, node: TsNode<'_>, source: &str) -> Option<String>;

    /// Determine visibility from AST node
    fn extract_visibility(&self, node: TsNode<'_>) -> Option<Visibility>;

    /// Check if function is async
    fn is_async(&self, node: TsNode<'_>) -> bool;

    /// Extract docstring from preceding comments
    fn extract_docstring(&self, node: TsNode<'_>, source: &str) -> Option<String>;
}

// Example: TypeScript extractor
pub struct TypeScriptExtractor;

impl LanguageExtractor for TypeScriptExtractor {
    fn function_types(&self) -> &[&str] {
        &["function_declaration", "arrow_function", "function_expression"]
    }

    fn class_types(&self) -> &[&str] {
        &["class_declaration", "abstract_class_declaration"]
    }

    fn method_types(&self) -> &[&str] {
        &["method_definition", "public_field_definition"]
    }

    // ... implementation
}
```

**TypeScript/JavaScript Decorator Extraction:**

```rust
impl TypeScriptExtractor {
    /// Extract decorators from a class, method, or property
    ///
    /// TypeScript decorators: @Component, @Injectable, @Get('/path')
    /// tree-sitter node type: "decorator"
    fn extract_decorators(&self, node: TsNode<'_>, source: &str) -> Vec<String> {
        let mut decorators = Vec::new();

        // Decorators are siblings before the decorated node
        if let Some(parent) = node.parent() {
            let mut cursor = parent.walk();
            for child in parent.children(&mut cursor) {
                if child.kind() == "decorator" {
                    // Extract the full decorator text: @Component({...})
                    let text = &source[child.start_byte()..child.end_byte()];
                    // Strip the @ prefix for storage
                    let decorator = text.trim_start_matches('@').to_string();
                    decorators.push(decorator);
                }
                // Stop when we reach the actual node
                if child.id() == node.id() {
                    break;
                }
            }
        }

        decorators
    }

    /// Create decorator edges for framework detection
    fn create_decorator_edges(&self, node_id: &NodeId, decorators: &[String]) -> Vec<Edge> {
        decorators.iter().filter_map(|dec| {
            // Parse decorator name: "Component({...})" -> "Component"
            let name = dec.split('(').next().unwrap_or(dec);

            Some(Edge {
                id: 0,
                source: NodeId(format!("decorator:{}", name)),
                target: node_id.clone(),
                kind: EdgeKind::Decorates,
                metadata: Some(serde_json::json!({ "full": dec })),
                line: None,
                col: None,
            })
        }).collect()
    }
}

/// Known TypeScript/JavaScript decorator patterns for framework detection
const TS_DECORATOR_PATTERNS: &[(&str, &str)] = &[
    // Angular
    ("Component", "angular"),
    ("Injectable", "angular"),
    ("NgModule", "angular"),
    ("Directive", "angular"),
    ("Pipe", "angular"),
    // NestJS
    ("Controller", "nestjs"),
    ("Get", "nestjs"),
    ("Post", "nestjs"),
    ("Put", "nestjs"),
    ("Delete", "nestjs"),
    ("Injectable", "nestjs"),
    ("Module", "nestjs"),
    // TypeORM
    ("Entity", "typeorm"),
    ("Column", "typeorm"),
    ("PrimaryGeneratedColumn", "typeorm"),
    ("ManyToOne", "typeorm"),
    ("OneToMany", "typeorm"),
    // MobX
    ("observable", "mobx"),
    ("action", "mobx"),
    ("computed", "mobx"),
    // Class-validator
    ("IsString", "class-validator"),
    ("IsNumber", "class-validator"),
    ("IsEmail", "class-validator"),
];
```

**Rust Attribute Macro Extraction:**

```rust
pub struct RustExtractor;

impl RustExtractor {
    /// Extract attribute macros from Rust code
    ///
    /// Rust attributes: #[derive(Debug)], #[test], #[tokio::main]
    /// tree-sitter node type: "attribute_item"
    fn extract_attributes(&self, node: TsNode<'_>, source: &str) -> Vec<String> {
        let mut attributes = Vec::new();

        // Attributes are children or siblings of the item
        let check_node = node.parent().unwrap_or(node);
        let mut cursor = check_node.walk();

        for child in check_node.children(&mut cursor) {
            if child.kind() == "attribute_item" {
                // Extract content: #[derive(Debug, Clone)] -> "derive(Debug, Clone)"
                let text = &source[child.start_byte()..child.end_byte()];
                // Strip #[ and ]
                let attr = text
                    .trim_start_matches("#[")
                    .trim_end_matches(']')
                    .to_string();
                attributes.push(attr);
            }
            // Stop at the actual node
            if child.id() == node.id() {
                break;
            }
        }

        attributes
    }

    /// Extract inner attributes (#![...]) for module-level metadata
    fn extract_inner_attributes(&self, node: TsNode<'_>, source: &str) -> Vec<String> {
        let mut attributes = Vec::new();
        let mut cursor = node.walk();

        for child in node.children(&mut cursor) {
            if child.kind() == "inner_attribute_item" {
                let text = &source[child.start_byte()..child.end_byte()];
                let attr = text
                    .trim_start_matches("#![")
                    .trim_end_matches(']')
                    .to_string();
                attributes.push(attr);
            }
        }

        attributes
    }

    /// Parse derive macros into individual traits
    fn parse_derive_traits(&self, attr: &str) -> Vec<String> {
        if attr.starts_with("derive(") {
            let inner = attr
                .trim_start_matches("derive(")
                .trim_end_matches(')');
            inner.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        } else {
            Vec::new()
        }
    }
}

/// Known Rust attribute patterns for framework/feature detection
const RUST_ATTRIBUTE_PATTERNS: &[(&str, &str)] = &[
    // Testing
    ("test", "test"),
    ("tokio::test", "tokio"),
    ("async_std::test", "async_std"),
    // Async runtimes
    ("tokio::main", "tokio"),
    ("async_std::main", "async_std"),
    // Serialization
    ("derive(Serialize", "serde"),
    ("derive(Deserialize", "serde"),
    ("serde(", "serde"),
    // Web frameworks
    ("get(", "actix-web"),
    ("post(", "actix-web"),
    ("route(", "axum"),
    // Macros
    ("derive(Debug", "std"),
    ("derive(Clone", "std"),
    ("derive(Default", "std"),
    // Proc macros
    ("proc_macro", "proc-macro"),
    ("proc_macro_derive", "proc-macro"),
];
```

**Dart/Flutter Extractor:**

```rust
pub struct DartExtractor {
    graphql_parser: Option<TreeSitterParser>,  // For embedded GraphQL
}

impl DartExtractor {
    /// Extract Flutter widget classes
    fn extract_widget(&self, node: TsNode<'_>, source: &str) -> Option<Node> {
        // Detect: class MyWidget extends StatelessWidget/StatefulWidget
        if node.kind() != "class_declaration" {
            return None;
        }

        let name = node.child_by_field_name("name")?;
        let superclass = node.child_by_field_name("superclass")?;
        let superclass_text = &source[superclass.start_byte()..superclass.end_byte()];

        let widget_type = match superclass_text {
            s if s.contains("StatelessWidget") => Some("stateless"),
            s if s.contains("StatefulWidget") => Some("stateful"),
            s if s.contains("State<") => Some("state"),
            s if s.contains("InheritedWidget") => Some("inherited"),
            s if s.contains("RenderObjectWidget") => Some("render"),
            _ => None,
        };

        if widget_type.is_some() {
            Some(Node {
                kind: NodeKind::Component,  // Widgets are components
                name: source[name.start_byte()..name.end_byte()].to_string(),
                // ... other fields
            })
        } else {
            None
        }
    }

    /// Extract Dart annotations (similar to TS decorators)
    fn extract_annotations(&self, node: TsNode<'_>, source: &str) -> Vec<String> {
        let mut annotations = Vec::new();

        if let Some(parent) = node.parent() {
            let mut cursor = parent.walk();
            for child in parent.children(&mut cursor) {
                if child.kind() == "annotation" {
                    let text = &source[child.start_byte()..child.end_byte()];
                    // Strip @ prefix
                    annotations.push(text.trim_start_matches('@').to_string());
                }
                if child.id() == node.id() {
                    break;
                }
            }
        }

        annotations
    }

    /// Extract embedded GraphQL queries (flutter_graphql, graphql_flutter)
    fn extract_embedded_graphql(&self, node: TsNode<'_>, source: &str) -> Vec<EmbeddedGraphQL> {
        let mut queries = Vec::new();

        // Pattern 1: gql('''...''') or gql("""...""")
        // Pattern 2: Query(document: gql(...))
        if node.kind() == "function_expression_invocation" {
            let function = node.child_by_field_name("function");
            if let Some(func) = function {
                let func_text = &source[func.start_byte()..func.end_byte()];
                if func_text == "gql" {
                    if let Some(args) = node.child_by_field_name("arguments") {
                        let content = self.extract_multiline_string(&args, source);
                        if let Some(gql_content) = content {
                            if let Some(ref parser) = self.graphql_parser {
                                queries.push(EmbeddedGraphQL {
                                    content: gql_content,
                                    host_line: node.start_position().row as u32,
                                    host_language: Language::Dart,
                                });
                            }
                        }
                    }
                }
            }
        }

        queries
    }
}

/// Known Dart/Flutter annotation patterns
const DART_ANNOTATION_PATTERNS: &[(&str, &str)] = &[
    // Core Dart
    ("override", "dart:core"),
    ("deprecated", "dart:core"),
    ("pragma", "dart:core"),

    // Flutter widgets
    ("immutable", "flutter"),
    ("required", "flutter"),
    ("protected", "flutter"),
    ("mustCallSuper", "flutter"),
    ("optionalTypeArgs", "flutter"),
    ("visibleForTesting", "flutter"),

    // Riverpod
    ("riverpod", "riverpod"),
    ("Riverpod", "riverpod"),
    ("ProviderScope", "riverpod"),

    // Bloc
    ("Bloc", "bloc"),
    ("Cubit", "bloc"),
    ("BlocProvider", "bloc"),
    ("BlocBuilder", "bloc"),
    ("BlocListener", "bloc"),

    // get_it DI
    ("injectable", "get_it"),
    ("singleton", "get_it"),
    ("lazySingleton", "get_it"),
    ("factoryMethod", "get_it"),
    ("preResolve", "get_it"),
    ("Injectable", "injectable"),
    ("Singleton", "injectable"),
    ("LazySingleton", "injectable"),

    // Freezed
    ("freezed", "freezed"),
    ("Freezed", "freezed"),
    ("JsonSerializable", "json_serializable"),

    // Routing
    ("GoRoute", "go_router"),
    ("TypedGoRoute", "go_router"),
    ("RoutePage", "auto_route"),
    ("AutoRoute", "auto_route"),
];

/// Flutter widget hierarchy detection
const FLUTTER_WIDGET_PATTERNS: &[(&str, &str)] = &[
    // Core widgets
    ("StatelessWidget", "widget"),
    ("StatefulWidget", "widget"),
    ("State", "state"),
    ("InheritedWidget", "inherited"),

    // Material
    ("MaterialApp", "app"),
    ("Scaffold", "layout"),
    ("AppBar", "layout"),

    // Routing
    ("Navigator", "navigation"),
    ("GoRouter", "navigation"),
    ("AutoRouter", "navigation"),

    // State management
    ("Provider", "provider"),
    ("ChangeNotifierProvider", "provider"),
    ("BlocProvider", "bloc"),
    ("ProviderScope", "riverpod"),
    ("ConsumerWidget", "riverpod"),
    ("HookConsumerWidget", "riverpod"),
];
```

**GraphQL Extractor (Standalone + Embedded):**

```rust
pub struct GraphQLExtractor;

impl GraphQLExtractor {
    /// Extract operations from GraphQL document
    fn extract_operations(&self, node: TsNode<'_>, source: &str) -> Vec<GraphQLOperation> {
        let mut operations = Vec::new();
        let mut cursor = node.walk();

        for child in node.children(&mut cursor) {
            match child.kind() {
                "operation_definition" => {
                    let op_type = child.child_by_field_name("type")
                        .map(|n| &source[n.start_byte()..n.end_byte()])
                        .unwrap_or("query");

                    let name = child.child_by_field_name("name")
                        .map(|n| source[n.start_byte()..n.end_byte()].to_string());

                    let variables = self.extract_variables(&child, source);
                    let selections = self.extract_selections(&child, source);

                    operations.push(GraphQLOperation {
                        kind: op_type.to_string(),
                        name,
                        variables,
                        selections,
                        start_line: child.start_position().row as u32,
                    });
                }
                "fragment_definition" => {
                    let name = child.child_by_field_name("name")
                        .map(|n| source[n.start_byte()..n.end_byte()].to_string());

                    let on_type = child.child_by_field_name("type_condition")
                        .map(|n| source[n.start_byte()..n.end_byte()].to_string());

                    operations.push(GraphQLOperation {
                        kind: "fragment".to_string(),
                        name,
                        variables: vec![],
                        selections: self.extract_selections(&child, source),
                        start_line: child.start_position().row as u32,
                    });
                }
                _ => {}
            }
        }

        operations
    }

    /// Extract field selections
    fn extract_selections(&self, node: &TsNode<'_>, source: &str) -> Vec<String> {
        let mut selections = Vec::new();

        if let Some(selection_set) = node.child_by_field_name("selection_set") {
            let mut cursor = selection_set.walk();
            for child in selection_set.children(&mut cursor) {
                if child.kind() == "field" {
                    if let Some(name) = child.child_by_field_name("name") {
                        selections.push(source[name.start_byte()..name.end_byte()].to_string());
                    }
                }
            }
        }

        selections
    }
}

#[derive(Debug, Clone)]
pub struct GraphQLOperation {
    pub kind: String,         // query, mutation, subscription, fragment
    pub name: Option<String>,
    pub variables: Vec<GraphQLVariable>,
    pub selections: Vec<String>,
    pub start_line: u32,
}

#[derive(Debug, Clone)]
pub struct GraphQLVariable {
    pub name: String,
    pub type_name: String,
    pub required: bool,
}

#[derive(Debug, Clone)]
pub struct EmbeddedGraphQL {
    pub content: String,
    pub host_line: u32,
    pub host_language: Language,
}

/// Embedded GraphQL detection for TypeScript/JavaScript
impl TypeScriptExtractor {
    /// Extract GraphQL from tagged template literals: gql`...`
    fn extract_embedded_graphql(&self, node: TsNode<'_>, source: &str) -> Vec<EmbeddedGraphQL> {
        let mut queries = Vec::new();

        if node.kind() == "tagged_template_expression" {
            let tag = node.child_by_field_name("tag");
            if let Some(tag) = tag {
                let tag_text = &source[tag.start_byte()..tag.end_byte()];
                if tag_text == "gql" || tag_text == "graphql" {
                    if let Some(template) = node.child_by_field_name("template_string") {
                        let content = &source[template.start_byte()..template.end_byte()];
                        // Strip backticks
                        let cleaned = content.trim_start_matches('`').trim_end_matches('`');
                        queries.push(EmbeddedGraphQL {
                            content: cleaned.to_string(),
                            host_line: node.start_position().row as u32,
                            host_language: Language::TypeScript,
                        });
                    }
                }
            }
        }

        queries
    }
}
```

**Bash/Shell Extractor:**

```rust
pub struct BashExtractor;

impl BashExtractor {
    /// Extract function definitions
    fn extract_function(&self, node: TsNode<'_>, source: &str) -> Option<Node> {
        // Pattern 1: function name() { }
        // Pattern 2: name() { }
        if node.kind() != "function_definition" {
            return None;
        }

        let name = node.child_by_field_name("name")?;
        let name_text = &source[name.start_byte()..name.end_byte()];

        Some(Node {
            kind: NodeKind::Function,
            name: name_text.to_string(),
            language: Language::Bash,
            // ... other fields
        })
    }

    /// Extract source/import statements
    fn extract_source(&self, node: TsNode<'_>, source: &str) -> Option<Edge> {
        // Pattern: source ./file.sh or . ./file.sh
        if node.kind() != "command" {
            return None;
        }

        let command_name = node.child_by_field_name("name")?;
        let cmd_text = &source[command_name.start_byte()..command_name.end_byte()];

        if cmd_text == "source" || cmd_text == "." {
            let arg = node.child_by_field_name("argument")?;
            let path = &source[arg.start_byte()..arg.end_byte()];
            return Some(Edge {
                kind: EdgeKind::Imports,
                // ... target is the sourced file
            });
        }

        None
    }

    /// Extract variable assignments
    fn extract_variable(&self, node: TsNode<'_>, source: &str) -> Option<Node> {
        if node.kind() != "variable_assignment" {
            return None;
        }

        let name = node.child_by_field_name("name")?;
        let name_text = &source[name.start_byte()..name.end_byte()];

        Some(Node {
            kind: NodeKind::Variable,
            name: name_text.to_string(),
            language: Language::Bash,
            // ... other fields
        })
    }
}
```

**Terraform/HCL Extractor:**

```rust
pub struct HclExtractor;

impl HclExtractor {
    /// Extract Terraform resource blocks
    fn extract_resource(&self, node: TsNode<'_>, source: &str) -> Option<Node> {
        // resource "aws_instance" "web" { }
        if node.kind() != "block" {
            return None;
        }

        let block_type = node.child(0)?;
        let type_text = &source[block_type.start_byte()..block_type.end_byte()];

        if type_text != "resource" {
            return None;
        }

        // Get resource type and name
        let resource_type = node.child(1)?;
        let resource_name = node.child(2)?;

        let type_str = source[resource_type.start_byte()..resource_type.end_byte()]
            .trim_matches('"');
        let name_str = source[resource_name.start_byte()..resource_name.end_byte()]
            .trim_matches('"');

        Some(Node {
            kind: NodeKind::Variable,  // or custom NodeKind::Resource
            name: format!("{}.{}", type_str, name_str),
            language: Language::Hcl,
            // ... metadata includes resource_type
        })
    }

    /// Extract module references
    fn extract_module(&self, node: TsNode<'_>, source: &str) -> Option<Node> {
        // module "vpc" { source = "..." }
        if node.kind() != "block" {
            return None;
        }

        let block_type = node.child(0)?;
        if &source[block_type.start_byte()..block_type.end_byte()] != "module" {
            return None;
        }

        let module_name = node.child(1)?;
        let name = source[module_name.start_byte()..module_name.end_byte()]
            .trim_matches('"');

        // Find source attribute for module path
        let source_path = self.find_attribute(&node, "source", source);

        Some(Node {
            kind: NodeKind::Module,
            name: name.to_string(),
            language: Language::Hcl,
            // ... metadata includes source_path
        })
    }

    /// Extract variable definitions
    fn extract_variable(&self, node: TsNode<'_>, source: &str) -> Option<Node> {
        // variable "region" { type = string, default = "us-west-2" }
        if node.kind() != "block" {
            return None;
        }

        let block_type = node.child(0)?;
        if &source[block_type.start_byte()..block_type.end_byte()] != "variable" {
            return None;
        }

        let var_name = node.child(1)?;
        let name = source[var_name.start_byte()..var_name.end_byte()]
            .trim_matches('"');

        Some(Node {
            kind: NodeKind::Variable,
            name: name.to_string(),
            language: Language::Hcl,
        })
    }

    /// Extract output definitions
    fn extract_output(&self, node: TsNode<'_>, source: &str) -> Option<Node> {
        // output "ip_address" { value = aws_instance.web.public_ip }
        if node.kind() != "block" {
            return None;
        }

        let block_type = node.child(0)?;
        if &source[block_type.start_byte()..block_type.end_byte()] != "output" {
            return None;
        }

        let output_name = node.child(1)?;
        let name = source[output_name.start_byte()..output_name.end_byte()]
            .trim_matches('"');

        Some(Node {
            kind: NodeKind::Export,  // Outputs are exports
            name: name.to_string(),
            language: Language::Hcl,
        })
    }

    /// Extract data source references
    fn extract_data(&self, node: TsNode<'_>, source: &str) -> Option<Node> {
        // data "aws_ami" "ubuntu" { }
        if node.kind() != "block" {
            return None;
        }

        let block_type = node.child(0)?;
        if &source[block_type.start_byte()..block_type.end_byte()] != "data" {
            return None;
        }

        let data_type = node.child(1)?;
        let data_name = node.child(2)?;

        let type_str = source[data_type.start_byte()..data_type.end_byte()]
            .trim_matches('"');
        let name_str = source[data_name.start_byte()..data_name.end_byte()]
            .trim_matches('"');

        Some(Node {
            kind: NodeKind::Variable,  // or custom NodeKind::DataSource
            name: format!("data.{}.{}", type_str, name_str),
            language: Language::Hcl,
        })
    }
}

/// Terraform block type patterns
const HCL_BLOCK_PATTERNS: &[(&str, &str)] = &[
    ("resource", "terraform"),
    ("data", "terraform"),
    ("module", "terraform"),
    ("variable", "terraform"),
    ("output", "terraform"),
    ("locals", "terraform"),
    ("provider", "terraform"),
    ("terraform", "terraform"),

    // Vault
    ("secret", "vault"),
    ("policy", "vault"),

    // Nomad
    ("job", "nomad"),
    ("group", "nomad"),
    ("task", "nomad"),
];
```

### 3.4 Graph Module (`codegraph-graph/`)

```rust
use std::collections::{HashMap, HashSet, VecDeque};

pub struct GraphTraverser<'a> {
    db: &'a DatabaseConnection,
}

#[derive(Debug, Clone)]
pub struct TraversalOptions {
    pub max_depth: usize,
    pub max_nodes: usize,
    pub direction: TraversalDirection,
    pub edge_kinds: Option<Vec<EdgeKind>>,
    pub node_kinds: Option<Vec<NodeKind>>,
    pub include_start: bool,
}

#[derive(Debug, Clone, Copy)]
pub enum TraversalDirection {
    Incoming,
    Outgoing,
    Both,
}

#[derive(Debug, Clone)]
pub struct Subgraph {
    pub nodes: HashMap<NodeId, Node>,
    pub edges: Vec<Edge>,
    pub roots: Vec<NodeId>,
}

impl<'a> GraphTraverser<'a> {
    pub fn traverse_bfs(&self, start: &NodeId, opts: &TraversalOptions) -> Result<Subgraph, GraphError> {
        let mut visited: HashSet<NodeId> = HashSet::new();
        let mut nodes: HashMap<NodeId, Node> = HashMap::new();
        let mut edges: Vec<Edge> = Vec::new();
        let mut queue: VecDeque<(NodeId, usize)> = VecDeque::new();

        // Get start node
        let start_node = self.db.get_node(start)?
            .ok_or(GraphError::NodeNotFound(start.clone()))?;

        if opts.include_start {
            nodes.insert(start.clone(), start_node.clone());
        }

        queue.push_back((start.clone(), 0));

        while let Some((current_id, depth)) = queue.pop_front() {
            if visited.contains(&current_id) {
                continue;
            }
            visited.insert(current_id.clone());

            if depth >= opts.max_depth || nodes.len() >= opts.max_nodes {
                continue;
            }

            let adjacent = self.db.get_adjacent_edges(&current_id, opts.direction)?;

            for edge in adjacent {
                // Filter by edge kind if specified
                if let Some(ref kinds) = opts.edge_kinds {
                    if !kinds.contains(&edge.kind) {
                        continue;
                    }
                }

                let target_id = match opts.direction {
                    TraversalDirection::Outgoing => &edge.target,
                    TraversalDirection::Incoming => &edge.source,
                    TraversalDirection::Both => {
                        if edge.source == current_id { &edge.target } else { &edge.source }
                    }
                };

                if !visited.contains(target_id) {
                    if let Some(target_node) = self.db.get_node(target_id)? {
                        // Filter by node kind if specified
                        if let Some(ref kinds) = opts.node_kinds {
                            if !kinds.contains(&target_node.kind) {
                                continue;
                            }
                        }

                        nodes.insert(target_id.clone(), target_node);
                        edges.push(edge);
                        queue.push_back((target_id.clone(), depth + 1));
                    }
                }
            }
        }

        Ok(Subgraph {
            nodes,
            edges,
            roots: vec![start.clone()],
        })
    }

    pub fn get_impact_radius(&self, node_id: &NodeId, max_depth: usize) -> Result<Subgraph, GraphError> {
        self.traverse_bfs(node_id, &TraversalOptions {
            max_depth,
            max_nodes: 100,
            direction: TraversalDirection::Incoming,
            edge_kinds: None,
            node_kinds: None,
            include_start: true,
        })
    }

    pub fn get_call_graph(&self, node_id: &NodeId, depth: usize) -> Result<Subgraph, GraphError> {
        let callers = self.traverse_bfs(node_id, &TraversalOptions {
            max_depth: depth,
            max_nodes: 50,
            direction: TraversalDirection::Incoming,
            edge_kinds: Some(vec![EdgeKind::Calls]),
            node_kinds: None,
            include_start: true,
        })?;

        let callees = self.traverse_bfs(node_id, &TraversalOptions {
            max_depth: depth,
            max_nodes: 50,
            direction: TraversalDirection::Outgoing,
            edge_kinds: Some(vec![EdgeKind::Calls]),
            node_kinds: None,
            include_start: false,
        })?;

        // Merge subgraphs
        Ok(self.merge_subgraphs(callers, callees))
    }
}
```

### 3.5 Vector/Embedding Module (`codegraph-vectors/`)

Uses direct [ort](https://ort.pyke.io/) (ONNX Runtime) for embeddings and [sqlite-vec](https://github.com/asg017/sqlite-vec) for vector search.

**Why direct `ort` instead of `rust-bert`:**
- **CoreML support**: Direct access to `ComputeUnits::CPUAndNeuralEngine` for M4 GPU/Neural Engine
- **Smaller binary**: No libtorch overhead
- **Full control**: Low-level ONNX inference with direct configuration
- **Simpler**: Embedding models don't need rust-bert's high-level NLP pipelines

On macOS, uses CoreML execution provider for GPU/Neural Engine acceleration.

```rust
use ort::{Session, GraphOptimizationLevel, inputs};
use ort::execution_providers::CoreMLExecutionProvider;
use tokenizers::Tokenizer;
use ndarray::{Array1, Array2, Axis};
use rusqlite::{ffi::sqlite3_auto_extension, Connection};
use sqlite_vec::sqlite3_vec_init;
use std::path::{Path, PathBuf};
use sha2::{Sha256, Digest};

/// Initialize sqlite-vec extension for the connection
///
/// # Security
/// - sqlite-vec is a pure C extension with no network access
/// - Runs entirely within the SQLite process
/// - No external dependencies at runtime
pub fn init_sqlite_vec() {
    unsafe {
        sqlite3_auto_extension(Some(std::mem::transmute(sqlite3_vec_init as *const ())));
    }
}

/// CRITICAL: This module runs entirely locally with no network access
pub struct TextEmbedder {
    session: Session,
    tokenizer: Tokenizer,
    embedding_dim: usize,
    model_config: EmbeddingModel,
}

impl TextEmbedder {
    /// Load model from local path ONLY
    ///
    /// # Security
    /// - Model must be pre-downloaded to ~/.codegraph/models/
    /// - No network requests are made
    /// - Model integrity verified via SHA-256 checksum
    pub fn load(model_dir: &Path, model_config: EmbeddingModel) -> Result<Self, EmbedderError> {
        // Verify model directory exists
        if !model_dir.exists() {
            return Err(EmbedderError::ModelNotFound(model_dir.to_path_buf()));
        }

        // Verify required files exist
        let model_file = model_dir.join("model.onnx");
        let tokenizer_file = model_dir.join("tokenizer.json");

        for file in [&model_file, &tokenizer_file] {
            if !file.exists() {
                return Err(EmbedderError::MissingModelFile(file.clone()));
            }
        }

        // Verify model checksum for security
        Self::verify_model_checksum(&model_file, model_config)?;

        // Load tokenizer
        let tokenizer = Tokenizer::from_file(&tokenizer_file)
            .map_err(|e| EmbedderError::TokenizerError(e.to_string()))?;

        // Configure ONNX session with platform-specific execution providers
        let session = Self::create_session(&model_file)?;

        Ok(Self {
            session,
            tokenizer,
            embedding_dim: model_config.dimension(),
            model_config,
        })
    }

    /// Create ONNX session with appropriate execution providers
    fn create_session(model_path: &Path) -> Result<Session, EmbedderError> {
        let mut builder = Session::builder()?
            .with_optimization_level(GraphOptimizationLevel::Level3)?
            .with_intra_threads(4)?;

        // macOS: Use CoreML for GPU/Neural Engine acceleration
        #[cfg(target_os = "macos")]
        {
            builder = builder.with_execution_providers([
                CoreMLExecutionProvider::default()
                    .with_subgraphs()
                    .with_ane_only(false)  // Use GPU + Neural Engine + CPU
                    .build()
            ])?;
        }

        builder.commit_from_file(model_path)
            .map_err(|e| EmbedderError::ModelLoadError(e.to_string()))
    }

    /// Verify model file checksum for security
    fn verify_model_checksum(model_path: &Path, model: EmbeddingModel) -> Result<(), EmbedderError> {
        let expected_checksum = model.expected_checksum();
        if expected_checksum.is_empty() {
            return Ok(()); // Skip verification if no checksum defined
        }

        let mut file = std::fs::File::open(model_path)?;
        let mut hasher = Sha256::new();
        std::io::copy(&mut file, &mut hasher)?;
        let actual = format!("{:x}", hasher.finalize());

        if actual != expected_checksum {
            return Err(EmbedderError::ChecksumMismatch {
                expected: expected_checksum.to_string(),
                actual,
            });
        }
        Ok(())
    }

    /// Generate embedding for text
    pub fn embed(&self, text: &str) -> Result<Vec<f32>, EmbedderError> {
        let results = self.embed_batch(&[text])?;
        Ok(results.into_iter().next().unwrap())
    }

    /// Batch embed multiple texts (more efficient than individual calls)
    pub fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbedderError> {
        // Apply prefixes if required by model
        let prefixed_texts: Vec<String> = texts
            .iter()
            .map(|t| format!("{}{}", self.model_config.document_prefix(), t))
            .collect();

        // Tokenize
        let encodings = self.tokenizer.encode_batch(
            prefixed_texts.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
            true,
        ).map_err(|e| EmbedderError::TokenizerError(e.to_string()))?;

        // Prepare inputs
        let input_ids: Vec<Vec<i64>> = encodings
            .iter()
            .map(|e| e.get_ids().iter().map(|&id| id as i64).collect())
            .collect();
        let attention_mask: Vec<Vec<i64>> = encodings
            .iter()
            .map(|e| e.get_attention_mask().iter().map(|&m| m as i64).collect())
            .collect();

        // Pad sequences to max length
        let max_len = input_ids.iter().map(|ids| ids.len()).max().unwrap_or(0);
        let batch_size = input_ids.len();

        let input_ids_array = Array2::from_shape_fn((batch_size, max_len), |(i, j)| {
            input_ids[i].get(j).copied().unwrap_or(0)
        });
        let attention_mask_array = Array2::from_shape_fn((batch_size, max_len), |(i, j)| {
            attention_mask[i].get(j).copied().unwrap_or(0)
        });

        // Run inference
        let outputs = self.session.run(inputs![
            "input_ids" => input_ids_array.view(),
            "attention_mask" => attention_mask_array.view(),
        ]?)?;

        // Extract embeddings (mean pooling over token dimension)
        let embeddings = outputs["last_hidden_state"]
            .try_extract_tensor::<f32>()?;

        // Mean pooling: average over sequence dimension
        let embeddings: Vec<Vec<f32>> = embeddings
            .axis_iter(Axis(0))
            .zip(attention_mask.iter())
            .map(|(seq_embeddings, mask)| {
                let mask_sum: f32 = mask.iter().map(|&m| m as f32).sum();
                seq_embeddings
                    .axis_iter(Axis(0))
                    .zip(mask.iter())
                    .fold(vec![0.0f32; self.embedding_dim], |mut acc, (token_emb, &m)| {
                        if m > 0 {
                            for (a, e) in acc.iter_mut().zip(token_emb.iter()) {
                                *a += e / mask_sum;
                            }
                        }
                        acc
                    })
            })
            .collect();

        Ok(embeddings)
    }

    /// Get the embedding dimension for this model
    pub fn embedding_dim(&self) -> usize {
        self.embedding_dim
    }
}

/// Supported embedding models (pre-downloadable)
///
/// Default: nomic-embed-text-v1.5 (matches TypeScript version)
/// - 768 dimensions
/// - 8192 token context window
/// - Supports: TypeScript, JavaScript, Rust, PHP, and 100+ other languages
#[derive(Debug, Clone, Copy, Default)]
pub enum EmbeddingModel {
    /// nomic-embed-text-v1.5: 768 dimensions, 8192 tokens, best for code + text
    /// Requires prefixes: "search_document: " and "search_query: "
    #[default]
    NomicEmbedTextV1_5,
    /// StarEncoder: 768 dimensions, code-optimized (for dual-embeddings feature)
    StarEncoder,
}

impl EmbeddingModel {
    pub fn dimension(&self) -> usize {
        768  // Both models use 768 dimensions
    }

    pub fn model_id(&self) -> &'static str {
        match self {
            Self::NomicEmbedTextV1_5 => "nomic-ai/nomic-embed-text-v1.5",
            Self::StarEncoder => "bigcode/starencoder",
        }
    }

    pub fn requires_prefix(&self) -> bool {
        matches!(self, Self::NomicEmbedTextV1_5)
    }

    pub fn document_prefix(&self) -> &'static str {
        match self {
            Self::NomicEmbedTextV1_5 => "search_document: ",
            Self::StarEncoder => "",
        }
    }

    pub fn query_prefix(&self) -> &'static str {
        match self {
            Self::NomicEmbedTextV1_5 => "search_query: ",
            Self::StarEncoder => "",
        }
    }
}

/// Vector search manager using sqlite-vec
pub struct VectorSearchManager {
    embedding_dim: usize,
}

impl VectorSearchManager {
    pub fn new(embedding_dim: usize) -> Self {
        Self { embedding_dim }
    }

    /// Create the vec0 virtual table for vector storage
    pub fn create_vector_table(&self, conn: &Connection) -> Result<(), VectorError> {
        conn.execute(
            &format!(
                "CREATE VIRTUAL TABLE IF NOT EXISTS vec_embeddings USING vec0(
                    node_id TEXT PRIMARY KEY,
                    embedding float[{}]
                )",
                self.embedding_dim
            ),
            [],
        )?;
        Ok(())
    }

    /// Insert or update a vector embedding
    pub fn upsert_embedding(
        &self,
        conn: &Connection,
        node_id: &str,
        embedding: &[f32],
    ) -> Result<(), VectorError> {
        // Delete existing if present (vec0 doesn't support upsert)
        conn.execute(
            "DELETE FROM vec_embeddings WHERE node_id = ?",
            [node_id],
        )?;

        // Insert new embedding using zerocopy for efficient byte conversion
        conn.execute(
            "INSERT INTO vec_embeddings(node_id, embedding) VALUES (?, ?)",
            rusqlite::params![node_id, embedding.as_bytes()],
        )?;
        Ok(())
    }

    /// Search for similar vectors using sqlite-vec's MATCH operator
    ///
    /// # Returns
    /// Vec of (node_id, distance) sorted by distance ascending
    pub fn search_similar(
        &self,
        conn: &Connection,
        query_embedding: &[f32],
        limit: usize,
    ) -> Result<Vec<(String, f64)>, VectorError> {
        let mut stmt = conn.prepare(
            "SELECT
                node_id,
                distance
            FROM vec_embeddings
            WHERE embedding MATCH ?1
            ORDER BY distance
            LIMIT ?2"
        )?;

        let results = stmt
            .query_map(
                rusqlite::params![query_embedding.as_bytes(), limit as i64],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?)),
            )?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(results)
    }

    /// Search with a distance threshold
    pub fn search_within_distance(
        &self,
        conn: &Connection,
        query_embedding: &[f32],
        max_distance: f64,
        limit: usize,
    ) -> Result<Vec<(String, f64)>, VectorError> {
        let mut stmt = conn.prepare(
            "SELECT
                node_id,
                distance
            FROM vec_embeddings
            WHERE embedding MATCH ?1
              AND distance < ?2
            ORDER BY distance
            LIMIT ?3"
        )?;

        let results = stmt
            .query_map(
                rusqlite::params![
                    query_embedding.as_bytes(),
                    max_distance,
                    limit as i64
                ],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?)),
            )?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(results)
    }

    /// Get embedding for a specific node
    pub fn get_embedding(
        &self,
        conn: &Connection,
        node_id: &str,
    ) -> Result<Option<Vec<f32>>, VectorError> {
        let mut stmt = conn.prepare(
            "SELECT embedding FROM vec_embeddings WHERE node_id = ?"
        )?;

        let result = stmt.query_row([node_id], |row| {
            let bytes: Vec<u8> = row.get(0)?;
            // Convert bytes back to f32 array
            let floats: Vec<f32> = bytes
                .chunks_exact(4)
                .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
                .collect();
            Ok(floats)
        });

        match result {
            Ok(embedding) => Ok(Some(embedding)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(VectorError::from(e)),
        }
    }

    /// Delete embedding for a node
    pub fn delete_embedding(&self, conn: &Connection, node_id: &str) -> Result<(), VectorError> {
        conn.execute(
            "DELETE FROM vec_embeddings WHERE node_id = ?",
            [node_id],
        )?;
        Ok(())
    }

    /// Get count of stored embeddings
    pub fn count_embeddings(&self, conn: &Connection) -> Result<usize, VectorError> {
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM vec_embeddings",
            [],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }
}
```

**Key sqlite-vec features:**
- **Pure C, no dependencies**: Runs anywhere SQLite runs
- **vec0 virtual table**: Optimized for vector storage and search
- **MATCH operator**: Fast brute-force similarity search
- **Distance filtering**: Support for `distance < threshold` in WHERE clause
- **Zero-copy with zerocopy crate**: Efficient f32 array to bytes conversion

### 3.5.1 Hybrid Embedding Strategy (Optional Feature: `dual-embeddings`)

When enabled, uses separate models for code and natural language:
- **Code Model**: StarEncoder (768 dims) - optimized for code structure and syntax
- **Text Model**: nomic-embed-text-v1.5 (768 dims) - optimized for natural language (comments, docstrings)

```
┌─────────────────────────────────────────────────────────────┐
│                       Code Node                              │
├─────────────────────────────────────────────────────────────┤
│  function parseConfig(path: string): Config {               │ ← Code Model
│    // Load and validate configuration from disk             │ ← Text Model
│    const data = fs.readFileSync(path);                      │ ← Code Model
│    return validate(JSON.parse(data));                       │ ← Code Model
│  }                                                          │
└─────────────────────────────────────────────────────────────┘
```

**Database Schema:**

```sql
-- Code embeddings (function bodies, signatures, identifiers)
CREATE VIRTUAL TABLE IF NOT EXISTS vec_code USING vec0(
    node_id TEXT PRIMARY KEY,
    embedding float[768]  -- StarEncoder
);

-- Text embeddings (comments, docstrings, natural language)
CREATE VIRTUAL TABLE IF NOT EXISTS vec_text USING vec0(
    node_id TEXT PRIMARY KEY,
    embedding float[768]  -- nomic-embed-text-v1.5
);
```

**Dual Embedder Implementation:**

```rust
/// Hybrid embedding manager with code and text models
pub struct DualEmbedder {
    code_model: SentenceEmbeddingsModel,  // StarEncoder
    text_model: SentenceEmbeddingsModel,  // nomic-embed-text
}

impl DualEmbedder {
    /// Load both models from local cache
    pub fn load(model_dir: &Path) -> Result<Self, EmbedderError> {
        let code_model = Self::load_model(&model_dir.join("starencoder"))?;
        let text_model = Self::load_model(&model_dir.join("nomic-embed-text-v1.5"))?;

        Ok(Self { code_model, text_model })
    }

    /// Embed code content (function bodies, expressions)
    pub fn embed_code(&self, code: &str) -> Result<Vec<f32>, EmbedderError> {
        self.code_model.encode(&[code])
            .map(|e| e.into_iter().next().unwrap())
            .map_err(|e| EmbedderError::InferenceError(e.to_string()))
    }

    /// Embed text content (comments, docstrings)
    pub fn embed_text(&self, text: &str) -> Result<Vec<f32>, EmbedderError> {
        // Use nomic prefix for document embedding
        let prefixed = format!("search_document: {}", text);
        self.text_model.encode(&[&prefixed])
            .map(|e| e.into_iter().next().unwrap())
            .map_err(|e| EmbedderError::InferenceError(e.to_string()))
    }

    /// Embed a query (auto-detects code vs natural language)
    pub fn embed_query(&self, query: &str) -> Result<QueryEmbedding, EmbedderError> {
        if Self::looks_like_code(query) {
            Ok(QueryEmbedding {
                code: Some(self.embed_code(query)?),
                text: None,
            })
        } else {
            let prefixed = format!("search_query: {}", query);
            Ok(QueryEmbedding {
                code: None,
                text: Some(self.text_model.encode(&[&prefixed])
                    .map(|e| e.into_iter().next().unwrap())
                    .map_err(|e| EmbedderError::InferenceError(e.to_string()))?),
            })
        }
    }

    /// Heuristic: does this look like code?
    fn looks_like_code(query: &str) -> bool {
        // Contains code patterns: dots, parens, brackets, operators
        let code_chars = ['.', '(', ')', '[', ']', '{', '}', ':', ';', '=', '-', '>'];
        let code_char_count = query.chars().filter(|c| code_chars.contains(c)).count();
        let has_camel_case = query.chars().any(|c| c.is_uppercase());

        code_char_count >= 2 || (has_camel_case && query.contains('.'))
    }
}

#[derive(Debug)]
pub struct QueryEmbedding {
    pub code: Option<Vec<f32>>,
    pub text: Option<Vec<f32>>,
}
```

**Hybrid Search Implementation:**

```rust
pub struct HybridSearchManager {
    code_dim: usize,
    text_dim: usize,
}

impl HybridSearchManager {
    /// Search using the appropriate index based on query type
    pub fn search(
        &self,
        conn: &Connection,
        query_embedding: &QueryEmbedding,
        limit: usize,
        weights: SearchWeights,
    ) -> Result<Vec<(String, f64)>, VectorError> {
        let mut results: HashMap<String, f64> = HashMap::new();

        // Search code index if we have code embedding
        if let Some(ref code_emb) = query_embedding.code {
            let code_results = self.search_index(conn, "vec_code", code_emb, limit * 2)?;
            for (node_id, distance) in code_results {
                let score = 1.0 / (1.0 + distance);
                *results.entry(node_id).or_default() += score * weights.code;
            }
        }

        // Search text index if we have text embedding
        if let Some(ref text_emb) = query_embedding.text {
            let text_results = self.search_index(conn, "vec_text", text_emb, limit * 2)?;
            for (node_id, distance) in text_results {
                let score = 1.0 / (1.0 + distance);
                *results.entry(node_id).or_default() += score * weights.text;
            }
        }

        // Sort by combined score
        let mut sorted: Vec<_> = results.into_iter().collect();
        sorted.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        sorted.truncate(limit);

        Ok(sorted)
    }

    fn search_index(
        &self,
        conn: &Connection,
        table: &str,
        embedding: &[f32],
        limit: usize,
    ) -> Result<Vec<(String, f64)>, VectorError> {
        let mut stmt = conn.prepare(&format!(
            "SELECT node_id, distance FROM {} WHERE embedding MATCH ?1 ORDER BY distance LIMIT ?2",
            table
        ))?;

        let results = stmt
            .query_map(rusqlite::params![embedding.as_bytes(), limit as i64], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(results)
    }
}

/// Weights for combining code and text search results
#[derive(Debug, Clone, Copy)]
pub struct SearchWeights {
    pub code: f64,
    pub text: f64,
}

impl Default for SearchWeights {
    fn default() -> Self {
        Self { code: 0.7, text: 0.3 }
    }
}
```

**Feature Flag Configuration:**

```toml
[features]
# Single model (default): nomic-embed-text-v1.5 for everything
vectors = ["rust-bert/onnx", "sqlite-vec", "zerocopy"]

# Dual model: StarEncoder for code + nomic-embed-text for comments
dual-embeddings = ["vectors"]
```

**CLI Configuration:**

```bash
# Enable dual embeddings during indexing
codegraph index --embeddings --dual-model

# Query with explicit mode
codegraph search "fs.readFileSync" --mode code
codegraph search "load configuration" --mode text
codegraph search "parseConfig" --mode auto  # default
```

**MCP Tool Extension:**

```rust
// In codegraph_context tool
{
    "embeddingMode": {
        "type": "string",
        "enum": ["auto", "code", "text"],
        "default": "auto",
        "description": "Embedding search mode: auto (detect), code (StarEncoder), text (nomic)"
    }
}
```

### 3.6 MCP Server (`codegraph-mcp/`)

```rust
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};

/// MCP Server using stdio transport
///
/// # Security
/// - Only accepts input from stdin
/// - Only outputs to stdout
/// - No network connections
/// - No file access outside project directory
pub struct MCPServer {
    codegraph: CodeGraph,
    root_dir: PathBuf,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "method")]
enum Request {
    #[serde(rename = "initialize")]
    Initialize { id: i64, params: InitializeParams },

    #[serde(rename = "tools/list")]
    ListTools { id: i64 },

    #[serde(rename = "tools/call")]
    CallTool { id: i64, params: ToolCallParams },
}

#[derive(Debug, Serialize)]
struct Response {
    jsonrpc: &'static str,
    id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

impl MCPServer {
    pub fn run(&mut self) -> Result<(), MCPError> {
        let stdin = std::io::stdin();
        let mut stdout = std::io::stdout();
        let reader = BufReader::new(stdin.lock());

        for line in reader.lines() {
            let line = line?;
            if line.is_empty() {
                continue;
            }

            let request: Request = serde_json::from_str(&line)?;
            let response = self.handle_request(request)?;

            let response_json = serde_json::to_string(&response)?;
            writeln!(stdout, "{}", response_json)?;
            stdout.flush()?;
        }

        Ok(())
    }

    fn handle_request(&mut self, request: Request) -> Result<Response, MCPError> {
        match request {
            Request::Initialize { id, params } => {
                // SECURITY: Validate root_uri is local path
                let root = PathBuf::from(&params.root_uri);
                if !root.is_absolute() || !root.exists() {
                    return Ok(Response::error(id, "Invalid root directory"));
                }

                self.root_dir = root;
                Ok(Response::ok(id, json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": "codegraph", "version": "1.0.0" }
                })))
            }

            Request::ListTools { id } => {
                Ok(Response::ok(id, json!({ "tools": TOOL_DEFINITIONS })))
            }

            Request::CallTool { id, params } => {
                let result = self.call_tool(&params.name, params.arguments)?;
                Ok(Response::ok(id, result))
            }
        }
    }
}
```

### 3.7 CLI (`codegraph-cli/`)

```rust
use clap::{Parser, Subcommand};
use indicatif::{ProgressBar, ProgressStyle};

#[derive(Parser)]
#[command(name = "codegraph")]
#[command(about = "Local-first code intelligence system")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize CodeGraph in a project
    Init {
        #[arg(default_value = ".")]
        path: PathBuf,
    },

    /// Full index of the codebase
    Index {
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Generate embeddings for semantic search
        #[arg(long)]
        embeddings: bool,
    },

    /// Incremental sync (changed files only)
    Sync {
        #[arg(default_value = ".")]
        path: PathBuf,
    },

    /// Show statistics
    Status {
        #[arg(default_value = ".")]
        path: PathBuf,
    },

    /// Search symbols
    Query {
        search: String,

        #[arg(short, long)]
        kind: Option<NodeKind>,

        #[arg(short, long, default_value = "10")]
        limit: usize,
    },

    /// Build context for AI
    Context {
        task: String,

        #[arg(long, default_value = "20")]
        max_nodes: usize,
    },

    /// Git hook management
    Hooks {
        #[command(subcommand)]
        action: HooksAction,
    },

    /// Start MCP server
    Serve {
        #[arg(long)]
        mcp: bool,
    },
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Index { path, embeddings }) => {
            let mut cg = CodeGraph::open(&path)?;

            let pb = ProgressBar::new(100);
            pb.set_style(ProgressStyle::default_bar()
                .template("{spinner:.green} [{bar:40}] {pos}% {msg}")?);

            cg.index_all(|progress| {
                pb.set_position(progress.percentage as u64);
                pb.set_message(progress.phase.clone());
            })?;

            if embeddings {
                cg.generate_embeddings(|progress| {
                    pb.set_position(progress.percentage as u64);
                    pb.set_message("Generating embeddings");
                })?;
            }

            pb.finish_with_message("Done");
        }

        // ... other commands

        None => {
            // Interactive installer
            run_installer()?;
        }
    }

    Ok(())
}
```

---

## 4. Dependency Mapping

### TypeScript to Rust Equivalents

| TypeScript Package | Rust Crate | Notes |
|--------------------|------------|-------|
| `better-sqlite3` | `rusqlite` | Same SQLite, different bindings |
| `sqlite-vss` (optional) | `sqlite-vec` | Vector search SQLite extension (successor to sqlite-vss) |
| `tree-sitter` | `tree-sitter` | Same C library |
| `tree-sitter-typescript` | `tree-sitter-typescript` | Same grammars |
| `@xenova/transformers` | `rust-bert` + `ort` | High-level NLP pipelines with ONNX backend |
| `commander` | `clap` | CLI parsing |
| `figlet` | `figlet-rs` | ASCII art |
| `crypto` (SHA256) | `sha2` | Hashing |
| `glob` | `glob` | File patterns |
| `chokidar` (unused) | `notify` | File watching |
| N/A | `serde` / `serde_json` | Serialization |
| N/A | `thiserror` | Error handling |
| N/A | `indicatif` | Progress bars |
| N/A | `rayon` | Parallelism |
| N/A | `zerocopy` | Zero-copy byte conversion for vectors |

### Cargo.toml Dependencies

```toml
[workspace.dependencies]
# Database
rusqlite = { version = "0.31", features = ["bundled", "vtab", "functions"] }

# Vector Search (sqlite-vec extension)
sqlite-vec = "0.1"
zerocopy = { version = "0.7", features = ["derive"] }

# Embeddings (rust-bert with ONNX backend, no network)
rust-bert = { version = "0.23", default-features = false, features = ["onnx"] }

# NOTE: ort is configured per-platform in [target] sections:
# - macOS: ort with load-dynamic + coreml (GPU/Neural Engine)
# - Linux/Windows: ort with load-dynamic only (CPU)
# See platform-specific dependencies below

# Tree-sitter core
tree-sitter = "0.22"

# Primary targets (first-class support)
tree-sitter-typescript = "0.21"
tree-sitter-javascript = "0.21"
tree-sitter-rust = "0.21"
tree-sitter-php = "0.21"
tree-sitter-dart = "0.0"           # Dart/Flutter
tree-sitter-graphql = "0.2"        # GraphQL (standalone + embedded)
tree-sitter-bash = "0.21"          # Shell scripts
tree-sitter-hcl = "1.1"            # Terraform, Vault, Nomad

# Secondary targets (extraction only)
tree-sitter-python = "0.21"
tree-sitter-go = "0.21"
tree-sitter-java = "0.21"
tree-sitter-c = "0.21"
tree-sitter-cpp = "0.21"
tree-sitter-c-sharp = "0.21"
tree-sitter-ruby = "0.21"
tree-sitter-swift = "0.21"
tree-sitter-kotlin = "0.21"

# CLI
clap = { version = "4.5", features = ["derive"] }
indicatif = "0.17"

# Serialization
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"

# Utilities
sha2 = "0.10"
glob = "0.3"
thiserror = "1.0"
rayon = "1.10"
walkdir = "2.5"

# Platform-specific ONNX runtime configuration
[target.'cfg(target_os = "macos")'.dependencies]
ort = { version = "2.0", features = ["load-dynamic", "coreml"] }

[target.'cfg(target_os = "linux")'.dependencies]
ort = { version = "2.0", features = ["load-dynamic"] }

[target.'cfg(target_os = "windows")'.dependencies]
ort = { version = "2.0", features = ["load-dynamic"] }
```

### ONNX Runtime Setup (No Network)

The `load-dynamic` feature requires pre-installing ONNX Runtime:

**macOS (with CoreML):**
```bash
# Download ONNX Runtime with CoreML support
curl -LO https://github.com/microsoft/onnxruntime/releases/download/v1.17.0/onnxruntime-osx-arm64-1.17.0.tgz
tar -xzf onnxruntime-osx-arm64-1.17.0.tgz
export ORT_DYLIB_PATH=/path/to/onnxruntime-osx-arm64-1.17.0/lib/libonnxruntime.dylib
```

**Linux:**
```bash
curl -LO https://github.com/microsoft/onnxruntime/releases/download/v1.17.0/onnxruntime-linux-x64-1.17.0.tgz
tar -xzf onnxruntime-linux-x64-1.17.0.tgz
export ORT_DYLIB_PATH=/path/to/onnxruntime-linux-x64-1.17.0/lib/libonnxruntime.so
```

**Windows:**
```powershell
# Download and extract onnxruntime-win-x64-1.17.0.zip
$env:ORT_DYLIB_PATH = "C:\path\to\onnxruntime.dll"
```

For distribution, bundle the ONNX Runtime library with the application or require users to install it separately.

---

## 5. Data Flow & Storage

### File Storage Structure

```
project-root/
├── .codegraph/
│   ├── config.json          # Configuration (JSON)
│   ├── codegraph.db         # SQLite database
│   ├── codegraph.db-wal     # WAL file (auto-managed)
│   ├── codegraph.db-shm     # Shared memory (auto-managed)
│   └── .gitignore           # Ignore DB files
│
~/.codegraph/                # Global directory
    └── models/              # Pre-downloaded embedding models
        ├── nomic-embed-text-v1.5/
        │   ├── model.onnx
        │   ├── tokenizer.json
        │   └── checksum.sha256
        └── ...
```

### Data Flow Diagram

```
┌──────────────────────────────────────────────────────────────┐
│                    LOCAL MACHINE BOUNDARY                     │
│                                                              │
│  ┌─────────────┐    ┌─────────────┐    ┌─────────────────┐  │
│  │ Source Code │───▶│  Extractor  │───▶│  SQLite DB      │  │
│  │    Files    │    │ (tree-sitter)│    │ (.codegraph/)   │  │
│  └─────────────┘    └─────────────┘    └────────┬────────┘  │
│                                                  │           │
│                     ┌────────────────────────────┘           │
│                     │                                        │
│                     ▼                                        │
│  ┌─────────────┐    ┌─────────────┐    ┌─────────────────┐  │
│  │  Embedder   │◀───│   Nodes     │───▶│  Graph Queries  │  │
│  │   (ONNX)    │    │   & Edges   │    │  (BFS/DFS)      │  │
│  └──────┬──────┘    └─────────────┘    └────────┬────────┘  │
│         │                                        │           │
│         ▼                                        ▼           │
│  ┌─────────────┐                       ┌─────────────────┐  │
│  │  Vectors    │                       │    Context      │  │
│  │  (SQLite)   │──────────────────────▶│    Builder      │  │
│  └─────────────┘                       └────────┬────────┘  │
│                                                  │           │
│                                                  ▼           │
│                                        ┌─────────────────┐  │
│                                        │   MCP Server    │  │
│                                        │   (stdio only)  │  │
│                                        └────────┬────────┘  │
│                                                  │           │
│                                                  ▼           │
│                                        ┌─────────────────┐  │
│                                        │     stdout      │  │
│                                        │  (to Claude)    │  │
│                                        └─────────────────┘  │
│                                                              │
│  ❌ NO NETWORK CONNECTIONS                                   │
│  ❌ NO EXTERNAL API CALLS                                    │
│  ❌ NO TELEMETRY                                             │
│  ❌ NO DATA EXFILTRATION                                     │
│                                                              │
└──────────────────────────────────────────────────────────────┘
```

---

## 6. Security Design

### 6.1 Network Isolation

```rust
/// COMPILE-TIME NETWORK RESTRICTION
///
/// This crate does NOT include any networking dependencies.
/// Attempting to add network code will fail at compile time.
///
/// The only I/O operations are:
/// 1. File system (read source code, write to .codegraph/)
/// 2. stdio (MCP server communication)

// Cargo.toml explicitly excludes:
// - reqwest, hyper, tokio (with net feature)
// - Any HTTP client libraries
// - Any socket libraries
```

### 6.2 Path Traversal Prevention

```rust
/// Validate that a path is within the project root
fn validate_path(root: &Path, path: &Path) -> Result<PathBuf, SecurityError> {
    let canonical_root = root.canonicalize()?;
    let canonical_path = path.canonicalize()?;

    if !canonical_path.starts_with(&canonical_root) {
        return Err(SecurityError::PathTraversal {
            attempted: path.to_path_buf(),
            root: root.to_path_buf(),
        });
    }

    Ok(canonical_path)
}

/// Validate file path before reading
fn safe_read_file(root: &Path, relative_path: &str) -> Result<String, SecurityError> {
    // Reject absolute paths
    if Path::new(relative_path).is_absolute() {
        return Err(SecurityError::AbsolutePathRejected(relative_path.to_string()));
    }

    // Reject path traversal attempts
    if relative_path.contains("..") {
        return Err(SecurityError::PathTraversalAttempt(relative_path.to_string()));
    }

    let full_path = root.join(relative_path);
    let validated = validate_path(root, &full_path)?;

    std::fs::read_to_string(validated)
        .map_err(|e| SecurityError::FileReadError(e))
}
```

### 6.3 Input Sanitization

```rust
/// Sanitize user input for SQL queries (defense in depth)
fn sanitize_search_query(query: &str) -> String {
    // Even with prepared statements, sanitize for FTS5
    query
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '_' || *c == '-')
        .collect()
}

/// Validate node ID format
fn validate_node_id(id: &str) -> Result<NodeId, ValidationError> {
    // Node IDs are: kind:32-char-hex
    let parts: Vec<&str> = id.split(':').collect();
    if parts.len() != 2 {
        return Err(ValidationError::InvalidNodeIdFormat(id.to_string()));
    }

    let kind = parts[0];
    let hash = parts[1];

    // Validate kind is known
    if NodeKind::from_str(kind).is_err() {
        return Err(ValidationError::InvalidNodeKind(kind.to_string()));
    }

    // Validate hash is 32 hex chars
    if hash.len() != 32 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(ValidationError::InvalidNodeIdHash(hash.to_string()));
    }

    Ok(NodeId(id.to_string()))
}
```

### 6.4 Model Integrity Verification

```rust
/// Verify model file integrity before loading
fn verify_model_integrity(model_dir: &Path) -> Result<(), SecurityError> {
    let checksum_file = model_dir.join("checksum.sha256");
    let model_file = model_dir.join("model.onnx");

    if !checksum_file.exists() {
        return Err(SecurityError::MissingChecksum);
    }

    let expected_hash = std::fs::read_to_string(&checksum_file)?
        .trim()
        .to_lowercase();

    let model_bytes = std::fs::read(&model_file)?;
    let actual_hash = {
        use sha2::{Sha256, Digest};
        let mut hasher = Sha256::new();
        hasher.update(&model_bytes);
        format!("{:x}", hasher.finalize())
    };

    if expected_hash != actual_hash {
        return Err(SecurityError::ModelIntegrityFailed {
            expected: expected_hash,
            actual: actual_hash,
        });
    }

    Ok(())
}
```

### 6.5 MCP Server Security

```rust
impl MCPServer {
    /// Handle tool call with security checks
    fn call_tool(&mut self, name: &str, args: serde_json::Value) -> Result<Value, MCPError> {
        // SECURITY: Only allow known tools
        let tool = match name {
            "codegraph_search" => self.handle_search(args),
            "codegraph_context" => self.handle_context(args),
            "codegraph_callers" => self.handle_callers(args),
            "codegraph_callees" => self.handle_callees(args),
            "codegraph_impact" => self.handle_impact(args),
            "codegraph_node" => self.handle_node(args),
            "codegraph_file_nodes" => self.handle_file_nodes(args),
            _ => return Err(MCPError::UnknownTool(name.to_string())),
        };

        tool
    }

    /// Handle codegraph_file_nodes - list all symbols in a file
    fn handle_file_nodes(&mut self, args: Value) -> Result<Value, MCPError> {
        let file_path: String = serde_json::from_value(args["filePath"].clone())?;

        // SECURITY: Validate file path is within project
        let validated_path = validate_path(&self.root_dir, &self.root_dir.join(&file_path))?;
        let relative_path = validated_path
            .strip_prefix(&self.root_dir)
            .unwrap_or(&validated_path)
            .to_string_lossy()
            .to_string();

        // Get all nodes in this file
        let nodes = self.codegraph.get_nodes_in_file(&relative_path)?;

        // Group by kind for organized output
        let mut by_kind: std::collections::HashMap<String, Vec<_>> = std::collections::HashMap::new();
        for node in &nodes {
            by_kind.entry(node.kind.to_string()).or_default().push(json!({
                "id": node.id.0,
                "name": node.name,
                "signature": node.signature,
                "startLine": node.start_line,
                "endLine": node.end_line,
                "visibility": node.visibility,
                "decorators": node.decorators,
            }));
        }

        Ok(json!({
            "filePath": relative_path,
            "nodeCount": nodes.len(),
            "nodes": by_kind,
        }))
    }

    fn handle_node(&mut self, args: Value) -> Result<Value, MCPError> {
        let node_id: String = serde_json::from_value(args["nodeId"].clone())?;

        // SECURITY: Validate node ID format
        let validated_id = validate_node_id(&node_id)?;

        let node = self.codegraph.get_node(&validated_id)?;

        // SECURITY: Only read code from within project
        let code = if let Some(ref node) = node {
            self.safe_read_source_code(&node.file_path, node.start_line, node.end_line)?
        } else {
            None
        };

        Ok(json!({ "node": node, "code": code }))
    }

    fn safe_read_source_code(
        &self,
        file_path: &str,
        start_line: u32,
        end_line: u32,
    ) -> Result<Option<String>, MCPError> {
        // SECURITY: Use validated path function
        let content = safe_read_file(&self.root_dir, file_path)?;

        let lines: Vec<&str> = content.lines().collect();
        let start = (start_line as usize).saturating_sub(1);
        let end = (end_line as usize).min(lines.len());

        if start >= lines.len() {
            return Ok(None);
        }

        Ok(Some(lines[start..end].join("\n")))
    }
}
```

---

## 7. Migration Strategy

### Phase 1: Core Foundation (Weeks 1-2)
- [ ] Set up Cargo workspace
- [ ] Implement `codegraph-db` with schema
- [ ] Implement core types in `codegraph/src/types.rs`
- [ ] Unit tests for database layer

### Phase 2: Extraction (Weeks 3-4)
- [ ] Implement tree-sitter parser wrapper
- [ ] Implement TypeScript extractor (most complex)
- [ ] Implement remaining language extractors
- [ ] Integration tests with sample codebases

### Phase 3: Graph (Week 5)
- [ ] Implement GraphTraverser
- [ ] Implement BFS/DFS algorithms
- [ ] Implement call graph and impact radius
- [ ] Test with known graph structures

### Phase 4: Resolution (Week 6)
- [ ] Implement import resolver
- [ ] Implement name matcher
- [ ] Implement framework-specific resolvers
- [ ] Integration tests

### Phase 5: Vectors (Week 7)
- [ ] Integrate `rust-bert` with ONNX backend
- [ ] Implement TextEmbedder using SentenceEmbeddingsModel
- [ ] Set up local model loading (no network)
- [ ] Integrate sqlite-vec extension
- [ ] Implement VectorSearchManager with vec0 virtual table
- [ ] Add embedding dimension validation
- [ ] Benchmark vector search against TypeScript version

### Phase 6: Context & MCP (Week 8)
- [ ] Implement ContextBuilder
- [ ] Implement MCP server
- [ ] End-to-end testing with Claude Code

### Phase 7: CLI & Polish (Week 9)
- [ ] Implement full CLI
- [ ] Progress reporting
- [ ] Error handling and messages
- [ ] Documentation

### Phase 8: Testing & Release (Week 10)
- [ ] Comprehensive test suite
- [ ] Performance benchmarks
- [ ] Security audit
- [ ] Cross-platform builds

---

## 8. Testing Strategy

### Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_node_id_generation() {
        let id = generate_node_id("src/main.rs", NodeKind::Function, "main", 10);
        assert!(id.0.starts_with("function:"));
        assert_eq!(id.0.len(), "function:".len() + 32);
    }

    #[test]
    fn test_path_traversal_prevention() {
        let root = TempDir::new().unwrap();

        // Should fail: path traversal attempt
        let result = validate_path(root.path(), &root.path().join("../etc/passwd"));
        assert!(result.is_err());

        // Should succeed: valid path within root
        std::fs::write(root.path().join("test.txt"), "content").unwrap();
        let result = validate_path(root.path(), &root.path().join("test.txt"));
        assert!(result.is_ok());
    }

    #[test]
    fn test_typescript_extraction() {
        let source = r#"
            export function greet(name: string): string {
                return `Hello, ${name}!`;
            }
        "#;

        let mut parser = TreeSitterParser::new();
        let result = parser.parse(source, Language::TypeScript).unwrap();

        assert_eq!(result.nodes.len(), 1);
        assert_eq!(result.nodes[0].kind, NodeKind::Function);
        assert_eq!(result.nodes[0].name, "greet");
    }
}
```

### Integration Tests

```rust
#[test]
fn test_full_indexing_workflow() {
    let temp = TempDir::new().unwrap();

    // Create sample project
    std::fs::write(
        temp.path().join("main.ts"),
        "export function main() { helper(); }\nfunction helper() {}",
    ).unwrap();

    // Initialize and index
    let mut cg = CodeGraph::init(temp.path()).unwrap();
    cg.index_all(|_| {}).unwrap();

    // Verify nodes created
    let nodes = cg.search("main", None, 10).unwrap();
    assert!(!nodes.is_empty());

    // Verify edges created
    let callees = cg.get_callees(&nodes[0].id).unwrap();
    assert!(callees.nodes.values().any(|n| n.name == "helper"));
}
```

### Security Tests

```rust
#[test]
fn test_no_network_access() {
    // This test verifies that no network dependencies are compiled
    // By checking Cargo.lock for networking crates
    let cargo_lock = std::fs::read_to_string("Cargo.lock").unwrap();

    let network_crates = ["reqwest", "hyper", "tokio/net", "socket2"];
    for crate_name in network_crates {
        assert!(
            !cargo_lock.contains(crate_name),
            "Network crate {} found in dependencies",
            crate_name
        );
    }
}

#[test]
fn test_model_checksum_required() {
    let temp = TempDir::new().unwrap();
    let model_dir = temp.path().join("models/test");
    std::fs::create_dir_all(&model_dir).unwrap();
    std::fs::write(model_dir.join("model.onnx"), "fake model").unwrap();

    // Should fail: no checksum file
    let result = verify_model_integrity(&model_dir);
    assert!(matches!(result, Err(SecurityError::MissingChecksum)));
}
```

---

## 9. Adversarial Scrutiny

This section provides a critical review of the plan, identifying gaps, conflicts, and areas where the TypeScript version may be superior. Each issue includes options for resolution.

### 9.1 Critical Issues Requiring Resolution

#### Issue 1: Model Configuration Conflicts

**Problem**: The plan references multiple embedding models inconsistently:
- Section 3.5 shows `AllMiniLmL6V2` (384 dims), `AllMiniLmL12V2` (384 dims), `AllDistilrobertaV1` (768 dims)
- Section 3.5.1 references `nomic-embed-text-v1.5` (768 dims) and `StarEncoder` (768 dims)
- The TypeScript version uses `nomic-ai/nomic-embed-text-v1.5` exclusively

**Options**:
| Option | Pros | Cons | Recommendation |
|--------|------|------|----------------|
| A: Use nomic-embed-text-v1.5 only | Matches TypeScript, simpler | Single model limits flexibility | ✅ **Default** |
| B: Support model selection | Flexible | More complex, testing burden | For future |
| C: Hardcode StarEncoder | Code-optimized | May not work well for natural language queries | Not recommended |

**Resolution**: Use `nomic-embed-text-v1.5` as the default single model. Remove `AllMiniLmL6V2` references. `dual-embeddings` feature adds StarEncoder as opt-in.

---

#### Issue 2: StarEncoder Availability in rust-bert

**Problem**: StarEncoder may not be available in rust-bert's pre-built model catalog. Need to verify ONNX model availability.

**Options**:
| Option | Pros | Cons | Recommendation |
|--------|------|------|----------------|
| A: Convert StarEncoder to ONNX manually | Works with rust-bert | Requires maintenance | For `dual-embeddings` feature |
| B: Use CodeBERT instead | Available in rust-bert | Doesn't support TypeScript/Rust well | Not recommended |
| C: Use nomic for both code and text | Simple | Slightly worse code search | ✅ **Default** |

**Resolution**: Default to nomic-only. Document StarEncoder ONNX conversion for `dual-embeddings` feature.

---

#### Issue 3: Missing MCP Tool Implementations

**Problem**: Only `codegraph_node` and `codegraph_file_nodes` have implementation details. The following tools lack implementation code:
- `codegraph_search`
- `codegraph_context`
- `codegraph_callers`
- `codegraph_callees`
- `codegraph_impact`

**Resolution**: Add implementation details in Section 9.6 below.

---

#### Issue 4: ContextBuilder Not Specified

**Problem**: TypeScript has a sophisticated `ContextBuilder` (434 lines) with:
- Semantic search fallback to text search
- Graph expansion from entry points
- Code block extraction with size limits
- Priority-based node selection
- Markdown/JSON formatting

The Rust plan only mentions "ContextBuilder" without implementation.

**Resolution**: Add ContextBuilder specification in Section 9.7 below.

---

#### Issue 5: Missing Types from TypeScript

**Problem**: The following TypeScript types are not defined in the Rust plan:
- `SearchResult` (node + score)
- `CodeBlock` (extracted source code)
- `TaskContext` (context building output)
- `BuildContextOptions`
- `FindRelevantContextOptions`

**Resolution**: Add type definitions in Section 9.8 below.

---

### 9.2 Completeness Issues

| Area | Issue | Resolution |
|------|-------|------------|
| **Liquid Language** | Not needed for target languages (TS, JS, Rust, PHP) | **REMOVED** - Add as future feature |
| **sqlite-vec pre-v1** | API may change | Pin `sqlite-vec = "0.1"`, monitor releases |
| **Node LRU Cache** | TypeScript has LRU cache in QueryBuilder | Add `lru` crate, cache `get_node` results |
| **Error Serialization** | Errors stored as JSON arrays | Use `serde_json::to_string(&errors)` |
| **Git Hooks Platform** | Need bash/batch scripts | Detect via `cfg!(windows)`, generate appropriate script |
| **Progress Callbacks** | TypeScript uses callbacks | Use `impl Fn(Progress)` or channel-based reporting |
| **Incremental Sync** | Change detection algorithm missing | Use file content hash comparison (already in TrackedFile) |

### 9.3 Accuracy Corrections

| Original Claim | Correction |
|----------------|------------|
| Multiple embedding models listed | Standardize on `nomic-embed-text-v1.5` (768 dims) |
| Node ID format "kind:32-char-hex" | Match TypeScript format: `"{kind}:{sha256_prefix}"` |
| `conn.prepare()` for caching | Use `conn.prepare_cached()` for LRU statement cache |
| Dimension from model config | Hardcode 768 for nomic, make configurable for custom models |

### 9.4 TypeScript Features Gap Analysis

| TypeScript Feature | Rust Plan Status | Action |
|--------------------|------------------|--------|
| Semantic → Text search fallback | ❌ Missing | Add fallback logic in search |
| Code block extraction with truncation | ❌ Missing | Add to ContextBuilder |
| Entry point prioritization | ❌ Missing | Add priority scoring |
| Subgraph merging | ❌ Missing | Add `merge_subgraphs()` |
| `get_nodes_in_file()` query | ✅ Added via `codegraph_file_nodes` | Complete |
| Decorator extraction (TS/JS) | ✅ Added | Complete |
| Attribute macro extraction (Rust) | ✅ Added | Complete |
| Framework pattern detection | ✅ Listed | Needs implementation detail |

### 9.5 Behavioral Parity Requirements

These must match TypeScript exactly for database compatibility:

| Behavior | Requirement |
|----------|-------------|
| Node ID hash | Same SHA256 input: `{filePath}:{kind}:{name}:{startLine}` |
| Line numbers | 1-indexed (not 0-indexed) |
| Path separators | Always forward slashes in database |
| FTS5 tokenization | Default SQLite tokenizer |
| Content hash | SHA256 of file content |

---

### 9.6 Missing Tool Implementations

```rust
impl MCPServer {
    /// codegraph_search - Quick symbol search by name
    fn handle_search(&mut self, args: Value) -> Result<Value, MCPError> {
        let query: String = serde_json::from_value(args["query"].clone())?;
        let kind: Option<String> = args.get("kind")
            .and_then(|k| k.as_str())
            .map(|s| s.to_string());
        let limit: usize = args.get("limit")
            .and_then(|l| l.as_u64())
            .unwrap_or(10) as usize;

        // Sanitize query for FTS5
        let sanitized = sanitize_search_query(&query);

        // Search using FTS5 + optional kind filter
        let results = self.codegraph.search_nodes(&sanitized, kind.as_deref(), limit)?;

        Ok(json!({
            "results": results.iter().map(|r| json!({
                "id": r.node.id.0,
                "name": r.node.name,
                "kind": r.node.kind,
                "filePath": r.node.file_path,
                "startLine": r.node.start_line,
                "score": r.score,
            })).collect::<Vec<_>>()
        }))
    }

    /// codegraph_context - Get relevant code context for a task
    fn handle_context(&mut self, args: Value) -> Result<Value, MCPError> {
        let task: String = serde_json::from_value(args["task"].clone())?;
        let max_nodes: usize = args.get("maxNodes")
            .and_then(|n| n.as_u64())
            .unwrap_or(20) as usize;
        let include_code: bool = args.get("includeCode")
            .and_then(|b| b.as_bool())
            .unwrap_or(true);
        let embedding_mode: &str = args.get("embeddingMode")
            .and_then(|m| m.as_str())
            .unwrap_or("auto");

        // Build context using ContextBuilder
        let context = self.codegraph.build_context(&task, BuildContextOptions {
            max_nodes,
            include_code,
            embedding_mode: embedding_mode.parse().unwrap_or_default(),
            ..Default::default()
        })?;

        Ok(json!({
            "summary": context.summary,
            "entryPoints": context.entry_points.iter().map(|n| json!({
                "id": n.id.0,
                "name": n.name,
                "kind": n.kind,
                "filePath": n.file_path,
            })).collect::<Vec<_>>(),
            "codeBlocks": context.code_blocks.iter().map(|b| json!({
                "filePath": b.file_path,
                "startLine": b.start_line,
                "endLine": b.end_line,
                "language": b.language,
                "content": b.content,
            })).collect::<Vec<_>>(),
            "stats": context.stats,
        }))
    }

    /// codegraph_callers - Find what calls a function
    fn handle_callers(&mut self, args: Value) -> Result<Value, MCPError> {
        let node_id: String = serde_json::from_value(args["nodeId"].clone())?;
        let depth: usize = args.get("depth")
            .and_then(|d| d.as_u64())
            .unwrap_or(1) as usize;
        let limit: usize = args.get("limit")
            .and_then(|l| l.as_u64())
            .unwrap_or(20) as usize;

        let validated_id = validate_node_id(&node_id)?;

        // Traverse incoming "calls" edges
        let subgraph = self.codegraph.traverser().traverse_bfs(&validated_id, &TraversalOptions {
            max_depth: depth,
            max_nodes: limit,
            direction: TraversalDirection::Incoming,
            edge_kinds: Some(vec![EdgeKind::Calls]),
            node_kinds: None,
            include_start: true,
        })?;

        Ok(json!({
            "nodeId": node_id,
            "callers": subgraph.nodes.values()
                .filter(|n| n.id != validated_id)
                .map(|n| json!({
                    "id": n.id.0,
                    "name": n.name,
                    "kind": n.kind,
                    "filePath": n.file_path,
                    "startLine": n.start_line,
                })).collect::<Vec<_>>(),
            "edges": subgraph.edges.iter().map(|e| json!({
                "source": e.source.0,
                "target": e.target.0,
                "line": e.line,
            })).collect::<Vec<_>>(),
        }))
    }

    /// codegraph_callees - Find what a function calls
    fn handle_callees(&mut self, args: Value) -> Result<Value, MCPError> {
        let node_id: String = serde_json::from_value(args["nodeId"].clone())?;
        let depth: usize = args.get("depth")
            .and_then(|d| d.as_u64())
            .unwrap_or(1) as usize;
        let limit: usize = args.get("limit")
            .and_then(|l| l.as_u64())
            .unwrap_or(20) as usize;

        let validated_id = validate_node_id(&node_id)?;

        // Traverse outgoing "calls" edges
        let subgraph = self.codegraph.traverser().traverse_bfs(&validated_id, &TraversalOptions {
            max_depth: depth,
            max_nodes: limit,
            direction: TraversalDirection::Outgoing,
            edge_kinds: Some(vec![EdgeKind::Calls]),
            node_kinds: None,
            include_start: true,
        })?;

        Ok(json!({
            "nodeId": node_id,
            "callees": subgraph.nodes.values()
                .filter(|n| n.id != validated_id)
                .map(|n| json!({
                    "id": n.id.0,
                    "name": n.name,
                    "kind": n.kind,
                    "filePath": n.file_path,
                    "startLine": n.start_line,
                })).collect::<Vec<_>>(),
            "edges": subgraph.edges.iter().map(|e| json!({
                "source": e.source.0,
                "target": e.target.0,
                "line": e.line,
            })).collect::<Vec<_>>(),
        }))
    }

    /// codegraph_impact - Analyze what code would be affected by changing a symbol
    fn handle_impact(&mut self, args: Value) -> Result<Value, MCPError> {
        let node_id: String = serde_json::from_value(args["nodeId"].clone())?;
        let depth: usize = args.get("depth")
            .and_then(|d| d.as_u64())
            .unwrap_or(2) as usize;

        let validated_id = validate_node_id(&node_id)?;

        // Get impact radius (all incoming references)
        let subgraph = self.codegraph.traverser().get_impact_radius(&validated_id, depth)?;

        // Group by file for easier understanding
        let mut by_file: HashMap<String, Vec<&Node>> = HashMap::new();
        for node in subgraph.nodes.values() {
            by_file.entry(node.file_path.clone()).or_default().push(node);
        }

        Ok(json!({
            "nodeId": node_id,
            "impactedNodes": subgraph.nodes.len() - 1,  // Exclude self
            "impactedFiles": by_file.len(),
            "byFile": by_file.iter().map(|(file, nodes)| json!({
                "filePath": file,
                "nodes": nodes.iter().map(|n| json!({
                    "id": n.id.0,
                    "name": n.name,
                    "kind": n.kind,
                    "startLine": n.start_line,
                })).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
            "directCallers": subgraph.edges.iter()
                .filter(|e| e.kind == EdgeKind::Calls && e.target == validated_id)
                .count(),
            "directReferences": subgraph.edges.iter()
                .filter(|e| e.kind == EdgeKind::References && e.target == validated_id)
                .count(),
        }))
    }
}
```

---

### 9.7 ContextBuilder Implementation

```rust
use std::collections::HashSet;

/// Options for building context
#[derive(Debug, Clone)]
pub struct BuildContextOptions {
    pub max_nodes: usize,
    pub max_code_blocks: usize,
    pub max_code_block_size: usize,
    pub include_code: bool,
    pub format: OutputFormat,
    pub search_limit: usize,
    pub traversal_depth: usize,
    pub min_score: f32,
    pub embedding_mode: EmbeddingMode,
}

impl Default for BuildContextOptions {
    fn default() -> Self {
        Self {
            max_nodes: 20,
            max_code_blocks: 5,
            max_code_block_size: 1500,
            include_code: true,
            format: OutputFormat::Markdown,
            search_limit: 3,
            traversal_depth: 1,
            min_score: 0.3,
            embedding_mode: EmbeddingMode::Auto,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub enum EmbeddingMode {
    #[default]
    Auto,
    Code,
    Text,
}

#[derive(Debug, Clone, Copy)]
pub enum OutputFormat {
    Markdown,
    Json,
    Structured,
}

/// Built context for a task
#[derive(Debug, Clone)]
pub struct TaskContext {
    pub query: String,
    pub subgraph: Subgraph,
    pub entry_points: Vec<Node>,
    pub code_blocks: Vec<CodeBlock>,
    pub related_files: Vec<String>,
    pub summary: String,
    pub stats: ContextStats,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContextStats {
    pub node_count: usize,
    pub edge_count: usize,
    pub file_count: usize,
    pub code_block_count: usize,
    pub total_code_size: usize,
}

/// A block of extracted source code
#[derive(Debug, Clone)]
pub struct CodeBlock {
    pub content: String,
    pub file_path: String,
    pub start_line: u32,
    pub end_line: u32,
    pub language: Language,
    pub node_id: NodeId,
}

/// Search result with score
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub node: Node,
    pub score: f32,
}

pub struct ContextBuilder<'a> {
    project_root: &'a Path,
    queries: &'a QueryBuilder,
    traverser: &'a GraphTraverser<'a>,
    vector_manager: Option<&'a VectorManager>,
}

impl<'a> ContextBuilder<'a> {
    /// Build context for a task query
    pub fn build_context(
        &self,
        query: &str,
        options: BuildContextOptions,
    ) -> Result<TaskContext, ContextError> {
        // 1. Find relevant context via search + traversal
        let subgraph = self.find_relevant_context(query, &options)?;

        // 2. Get entry points (search result nodes)
        let entry_points: Vec<Node> = subgraph.roots.iter()
            .filter_map(|id| subgraph.nodes.get(id).cloned())
            .collect();

        // 3. Extract code blocks for key nodes
        let code_blocks = if options.include_code {
            self.extract_code_blocks(&subgraph, options.max_code_blocks, options.max_code_block_size)?
        } else {
            vec![]
        };

        // 4. Get related files
        let related_files: Vec<String> = subgraph.nodes.values()
            .map(|n| n.file_path.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();

        // 5. Generate summary
        let summary = self.generate_summary(query, &subgraph, &entry_points);

        // 6. Calculate stats
        let stats = ContextStats {
            node_count: subgraph.nodes.len(),
            edge_count: subgraph.edges.len(),
            file_count: related_files.len(),
            code_block_count: code_blocks.len(),
            total_code_size: code_blocks.iter().map(|b| b.content.len()).sum(),
        };

        Ok(TaskContext {
            query: query.to_string(),
            subgraph,
            entry_points,
            code_blocks,
            related_files,
            summary,
            stats,
        })
    }

    /// Find relevant subgraph using semantic search with text fallback
    fn find_relevant_context(
        &self,
        query: &str,
        options: &BuildContextOptions,
    ) -> Result<Subgraph, ContextError> {
        let mut search_results: Vec<SearchResult> = vec![];

        // Try semantic search first
        if let Some(vm) = self.vector_manager {
            if let Ok(results) = vm.search(query, options.search_limit) {
                search_results = results;
            }
        }

        // Fallback to text search if semantic search yielded nothing
        if search_results.is_empty() {
            search_results = self.queries.search_nodes(query, options.search_limit)?
                .into_iter()
                .map(|node| SearchResult { node, score: 1.0 })
                .collect();
        }

        // Filter by minimum score
        let filtered: Vec<_> = search_results.into_iter()
            .filter(|r| r.score >= options.min_score)
            .collect();

        // Build subgraph from search results + traversal
        let mut nodes = HashMap::new();
        let mut edges = Vec::new();
        let mut roots = Vec::new();

        for result in &filtered {
            nodes.insert(result.node.id.clone(), result.node.clone());
            roots.push(result.node.id.clone());

            // Traverse from each entry point
            let traversal = self.traverser.traverse_bfs(&result.node.id, &TraversalOptions {
                max_depth: options.traversal_depth,
                max_nodes: options.max_nodes / filtered.len().max(1),
                direction: TraversalDirection::Both,
                edge_kinds: None,
                node_kinds: None,
                include_start: false,
            })?;

            // Merge results
            for (id, node) in traversal.nodes {
                nodes.entry(id).or_insert(node);
            }
            for edge in traversal.edges {
                if !edges.iter().any(|e: &Edge| e.source == edge.source && e.target == edge.target) {
                    edges.push(edge);
                }
            }
        }

        // Trim to max_nodes, prioritizing entry points
        if nodes.len() > options.max_nodes {
            nodes = self.prioritize_nodes(nodes, &roots, &edges, options.max_nodes);
            edges.retain(|e| nodes.contains_key(&e.source) && nodes.contains_key(&e.target));
        }

        Ok(Subgraph { nodes, edges, roots })
    }

    /// Prioritize nodes: entry points first, then their neighbors
    fn prioritize_nodes(
        &self,
        nodes: HashMap<NodeId, Node>,
        roots: &[NodeId],
        edges: &[Edge],
        max: usize,
    ) -> HashMap<NodeId, Node> {
        let mut priority_ids: HashSet<NodeId> = roots.iter().cloned().collect();

        // Add direct neighbors of roots
        for edge in edges {
            if priority_ids.contains(&edge.source) {
                priority_ids.insert(edge.target.clone());
            }
            if priority_ids.contains(&edge.target) {
                priority_ids.insert(edge.source.clone());
            }
        }

        let mut result = HashMap::new();

        // Add priority nodes first
        for id in &priority_ids {
            if result.len() >= max { break; }
            if let Some(node) = nodes.get(id) {
                result.insert(id.clone(), node.clone());
            }
        }

        // Fill remaining slots
        for (id, node) in &nodes {
            if result.len() >= max { break; }
            result.entry(id.clone()).or_insert(node.clone());
        }

        result
    }

    /// Extract code blocks, prioritizing entry points and functions
    fn extract_code_blocks(
        &self,
        subgraph: &Subgraph,
        max_blocks: usize,
        max_size: usize,
    ) -> Result<Vec<CodeBlock>, ContextError> {
        let mut blocks = Vec::new();
        let mut seen_files: HashSet<String> = HashSet::new();

        // Priority order: roots → functions/methods → classes
        let mut priority_nodes: Vec<&Node> = vec![];

        for id in &subgraph.roots {
            if let Some(node) = subgraph.nodes.get(id) {
                priority_nodes.push(node);
            }
        }

        for node in subgraph.nodes.values() {
            if !subgraph.roots.contains(&node.id) {
                if matches!(node.kind, NodeKind::Function | NodeKind::Method) {
                    priority_nodes.push(node);
                }
            }
        }

        for node in subgraph.nodes.values() {
            if !subgraph.roots.contains(&node.id) && node.kind == NodeKind::Class {
                priority_nodes.push(node);
            }
        }

        for node in priority_nodes {
            if blocks.len() >= max_blocks { break; }

            // Avoid duplicate files in code blocks
            if seen_files.contains(&node.file_path) { continue; }

            if let Ok(code) = self.read_node_code(node, max_size) {
                seen_files.insert(node.file_path.clone());
                blocks.push(CodeBlock {
                    content: code,
                    file_path: node.file_path.clone(),
                    start_line: node.start_line,
                    end_line: node.end_line,
                    language: node.language,
                    node_id: node.id.clone(),
                });
            }
        }

        Ok(blocks)
    }

    /// Read and optionally truncate node source code
    fn read_node_code(&self, node: &Node, max_size: usize) -> Result<String, ContextError> {
        let file_path = self.project_root.join(&node.file_path);
        let content = safe_read_file(self.project_root, &node.file_path)?;

        let lines: Vec<&str> = content.lines().collect();
        let start = (node.start_line as usize).saturating_sub(1);
        let end = (node.end_line as usize).min(lines.len());

        let code = lines[start..end].join("\n");

        if code.len() > max_size {
            Ok(format!("{}\n// ... truncated ...", &code[..max_size]))
        } else {
            Ok(code)
        }
    }

    fn generate_summary(&self, query: &str, subgraph: &Subgraph, entry_points: &[Node]) -> String {
        let names: Vec<_> = entry_points.iter().take(3).map(|n| n.name.as_str()).collect();
        let remaining = if entry_points.len() > 3 {
            format!(" and {} more", entry_points.len() - 3)
        } else {
            String::new()
        };

        let files: HashSet<_> = subgraph.nodes.values().map(|n| &n.file_path).collect();

        format!(
            "Found {} relevant code symbols across {} files. Key entry points: {}{}. {} relationships identified.",
            subgraph.nodes.len(),
            files.len(),
            names.join(", "),
            remaining,
            subgraph.edges.len()
        )
    }
}
```

---

### 9.8 Database Query Layer Additions

```rust
impl QueryBuilder {
    /// Search nodes by name using FTS5
    pub fn search_nodes(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<Node>, DbError> {
        let sanitized = sanitize_search_query(query);

        let mut stmt = self.conn.prepare_cached(
            "SELECT n.* FROM nodes n
             JOIN nodes_fts f ON n.rowid = f.rowid
             WHERE nodes_fts MATCH ?1
             ORDER BY rank
             LIMIT ?2"
        )?;

        let nodes = stmt.query_map([&sanitized, &limit.to_string()], |row| {
            self.row_to_node(row)
        })?.collect::<Result<Vec<_>, _>>()?;

        Ok(nodes)
    }

    /// Get all nodes in a file
    pub fn get_nodes_in_file(&self, file_path: &str) -> Result<Vec<Node>, DbError> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT * FROM nodes WHERE file_path = ? ORDER BY start_line"
        )?;

        let nodes = stmt.query_map([file_path], |row| {
            self.row_to_node(row)
        })?.collect::<Result<Vec<_>, _>>()?;

        Ok(nodes)
    }

    /// Merge two subgraphs, avoiding duplicate edges
    pub fn merge_subgraphs(&self, a: Subgraph, b: Subgraph) -> Subgraph {
        let mut nodes = a.nodes;
        let mut edges = a.edges;
        let mut roots = a.roots;

        for (id, node) in b.nodes {
            nodes.entry(id).or_insert(node);
        }

        for edge in b.edges {
            let exists = edges.iter().any(|e|
                e.source == edge.source && e.target == edge.target && e.kind == edge.kind
            );
            if !exists {
                edges.push(edge);
            }
        }

        for root in b.roots {
            if !roots.contains(&root) {
                roots.push(root);
            }
        }

        Subgraph { nodes, edges, roots }
    }
}
```

---

### 9.9 Future Enhancements (Not in Initial Release)

The following features are deferred to future releases:

| Feature | Reason | Priority |
|---------|--------|----------|
| **Liquid language support** | Not in primary targets | Low |
| **Context7 integration** | Requires network access | Medium |
| **Doc tool** | Separate feature for PRDs, user stories | Medium |
| **int8/binary quantization** | Optimization after baseline works | Low |
| **Multiple embedding models** | Complexity; single model works well | Low |
| **Python framework patterns** | Secondary target, extraction only | Low |
| **Go framework patterns** | Secondary target, extraction only | Low |

### 9.10 Primary Target Languages

The following languages have **first-class support** with full framework patterns:

| Language | Frameworks/Patterns |
|----------|---------------------|
| **TypeScript/JavaScript** | React, Next.js, Express, NestJS, Angular, embedded GraphQL |
| **Rust** | Actix-web, Axum, Tokio, Serde, attribute macros |
| **PHP** | Laravel, Symfony |
| **Dart/Flutter** | Widgets, Riverpod, Bloc, get_it DI, GoRouter, AutoRoute, embedded GraphQL |
| **GraphQL** | Operations, fragments, embedded in TS/JS/Dart |
| **Bash** | Functions, source imports, variables |
| **Terraform/HCL** | Resources, modules, variables, outputs, data sources |

---

## 10. Security Audit

### 10.1 Attack Surface Analysis

| Surface | Risk | Mitigation |
|---------|------|------------|
| **File System Read** | Path traversal to read sensitive files | `validate_path()` with canonicalization |
| **File System Write** | Overwrite critical files | Only write to `.codegraph/` directory |
| **SQL Injection** | Malicious search queries | Prepared statements exclusively |
| **FTS5 Injection** | Crafted FTS queries | Sanitize FTS input, limit query complexity |
| **sqlite-vec queries** | Malformed vector data | Validate embedding dimensions, use zerocopy safely |
| **Model Loading** | Malicious ONNX model | Checksum verification required |
| **MCP Input** | Malformed JSON-RPC | Strict schema validation with serde |
| **Memory Exhaustion** | Large files or deep traversals | Configurable limits on file size and depth |
| **Vector Storage DoS** | Many large embeddings | Limit total embedding count, consider quantization |
| **Symlink Following** | Escape project via symlinks | Use `canonicalize()` and recheck bounds |
| **Race Conditions** | TOCTOU in path validation | Use file handles, not paths where possible |

### 10.1.1 sqlite-vec Security Notes

sqlite-vec is a **pure C extension with no dependencies**, which provides several security benefits:

1. **No network access**: sqlite-vec cannot make network requests
2. **Memory safe (within SQLite)**: Uses SQLite's memory allocator
3. **No file access**: Only operates on data passed to it via SQL
4. **Deterministic**: Same inputs always produce same outputs

**Potential risks:**
- Pre-v1 software: API may have undiscovered bugs
- Vector dimension mismatch could cause issues (mitigated by validation)
- Large result sets could consume memory (mitigated by LIMIT clauses)

### 10.2 Data Exfiltration Prevention

```
┌─────────────────────────────────────────────────────────────┐
│                    EXFILTRATION VECTORS                      │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  ❌ Network Sockets        - No networking crates           │
│  ❌ HTTP Requests          - No HTTP client crates          │
│  ❌ DNS Queries            - No DNS resolution              │
│  ❌ External Processes     - No shell execution with data   │
│  ❌ File System (outside)  - Path validation enforced       │
│  ❌ Environment Variables  - Not used for data storage      │
│  ❌ Clipboard              - No clipboard access            │
│  ❌ IPC (except stdio)     - Only MCP via stdio             │
│                                                             │
│  ✅ ALLOWED: Local file I/O within project                  │
│  ✅ ALLOWED: stdio for MCP protocol                         │
│  ✅ ALLOWED: SQLite database in .codegraph/                 │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

### 10.3 Specific Vulnerability Checks

#### SQL Injection

```rust
// VULNERABLE (DO NOT USE):
let query = format!("SELECT * FROM nodes WHERE name = '{}'", user_input);
conn.execute(&query, [])?;

// SAFE (ALWAYS USE):
let mut stmt = conn.prepare_cached("SELECT * FROM nodes WHERE name = ?")?;
stmt.query_row([user_input], |row| { ... })?;
```

#### Path Traversal

```rust
// VULNERABLE:
let path = root.join(user_path);  // user_path could be "../../../etc/passwd"
std::fs::read_to_string(path)?;

// SAFE:
let path = validate_path(&root, &root.join(user_path))?;  // Canonicalizes and checks bounds
std::fs::read_to_string(path)?;
```

#### Command Injection (Git Hooks)

```rust
// VULNERABLE:
let script = format!("codegraph sync {}", project_path);  // path could contain `; rm -rf /`

// SAFE:
let script = format!(
    "codegraph sync {:?}",  // {:?} adds quotes and escapes
    project_path.display()
);
// Better: Use absolute path and validate it's a directory
```

#### Denial of Service

```rust
// VULNERABLE:
fn traverse_all(&self, start: NodeId) -> Vec<Node> {
    // No depth limit - could traverse forever on cyclic graph
}

// SAFE:
fn traverse_all(&self, start: NodeId, opts: &TraversalOptions) -> Vec<Node> {
    // opts.max_depth and opts.max_nodes enforced
}
```

### 10.4 Threat Model

| Threat Actor | Capability | Mitigated By |
|--------------|------------|--------------|
| Malicious codebase | Contains crafted filenames/content | Path validation, input sanitization |
| Malicious Claude prompt | Tries to exfiltrate via MCP | No network capability, path bounds |
| Compromised model file | ONNX model with malicious ops | Checksum verification, no network in ONNX |
| Local privilege escalation | Symlinks to sensitive files | Canonicalization, bounds checking |

### 10.5 Recommended Security Hardening

1. **Sandboxing**: Consider running embedding inference in a sandboxed subprocess
2. **Seccomp** (Linux): Restrict syscalls to file I/O and computation only
3. **Memory Limits**: Use `rlimit` to prevent memory exhaustion attacks
4. **Audit Logging**: Log all file accesses for forensic analysis
5. **Fuzzing**: Fuzz test all input parsing (JSON, SQL, file paths)

---

## 11. Clarifying Questions

Before proceeding with implementation, the following questions should be resolved:

### Architecture Questions

1. **Async vs Sync**: Should the Rust version use async (tokio) for I/O, or stay synchronous like the TypeScript version? Async adds complexity but may improve MCP server responsiveness.

2. **sqlite-vec quantization**: sqlite-vec supports int8 and binary quantization for smaller storage. Should the Rust version:
   - Use float32 only (maximum accuracy, ~3KB per embedding)?
   - Support int8 quantization (8x smaller, slight accuracy loss)?
   - Support binary quantization (32x smaller, more accuracy loss)?

3. **Cross-Platform Priority**: What platforms must be supported at launch?
   - Linux x64 (CI/server environments)
   - macOS x64 and ARM64 (developer machines)
   - Windows x64 (developer machines)

### Feature Questions

4. **Embedding Model**: Should the Rust version:
   - Ship with a bundled model (larger binary, easier setup)?
   - Require users to download the model separately (smaller binary, more setup)?
   - Support multiple models (more flexible, more complex)?

5. **MCP Protocol Version**: Which MCP protocol version should be targeted?
   - The TypeScript version uses `2024-11-05`. Is this still current?

6. **Backward Compatibility**: Should the Rust version be able to read databases created by the TypeScript version?
   - If yes, need schema compatibility testing
   - If no, can optimize schema for Rust

### Security Questions

7. **Model Download**: If users need to download embedding models, how should this be handled securely?
   - Provide a separate `codegraph model download` command that does network access?
   - Require manual download with checksum verification?
   - Ship models with the binary?

8. **Git Hook Security**: Git hooks run arbitrary code. Should the Rust version:
   - Generate hooks that validate the codegraph binary before running?
   - Use a hook that calls codegraph with specific arguments only?

9. **Audit Trail**: Should file accesses be logged for security auditing?
   - If yes, where should logs be stored?
   - What information should be logged?

### Implementation Questions

10. **Error Handling Strategy**: Should errors be:
    - Propagated up with full context (more helpful, larger binary)?
    - Simplified to error codes (smaller binary, less helpful)?

11. **Testing Parity**: Should there be tests that verify identical output between TypeScript and Rust versions?
    - This ensures migration doesn't break existing workflows
    - Requires maintaining both codebases during transition

12. **Release Strategy**: Should the Rust version:
    - Replace the TypeScript version entirely?
    - Coexist as `codegraph-rs` alongside the TypeScript version?
    - Be a drop-in replacement with the same binary name?

---

## Appendix A: Security Checklist

Before each release, verify:

- [ ] No networking crates in `Cargo.lock`
- [ ] All file paths validated before access
- [ ] All SQL uses prepared statements
- [ ] Model checksum verified before loading
- [ ] Symlinks resolved and bounds-checked
- [ ] MCP input validated against schema
- [ ] Traversal depth limits enforced
- [ ] File size limits enforced
- [ ] Memory limits tested
- [ ] Fuzz tests pass without crashes

---

## Appendix B: File Format Compatibility

### config.json

```json
{
  "version": 1,
  "rootDir": "/path/to/project",
  "include": ["**/*.ts", "**/*.js"],
  "exclude": ["node_modules/**", "dist/**"],
  "languages": [],
  "frameworks": ["react"],
  "maxFileSize": 1048576,
  "extractDocstrings": true,
  "trackCallSites": true,
  "enableEmbeddings": true
}
```

### Database Schema Version

- Current version: 1
- Rust version should support reading version 1 databases
- Future schema changes should use migrations

---

*Document Version: 1.0*
*Created: 2025-01-31*
*Author: Claude (Rust Rewrite Planning)*
