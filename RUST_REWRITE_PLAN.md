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
│   │       │   ├── typescript.rs
│   │       │   ├── python.rs
│   │       │   ├── rust.rs
│   │       │   ├── go.rs
│   │       │   └── ... (12 more)
│   │       └── grammars.rs       # Grammar loading
│   │
│   ├── codegraph-resolution/     # Reference resolution
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── resolver.rs       # ReferenceResolver
│   │       ├── imports.rs        # Import path resolution
│   │       ├── names.rs          # Name matching
│   │       └── frameworks/       # Framework-specific
│   │           ├── mod.rs
│   │           ├── react.rs
│   │           ├── express.rs
│   │           └── ... (7 more)
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
vectors = ["rust-bert/onnx", "sqlite-vec", "zerocopy"]  # Base: embeddings + vector search
vectors-coreml = ["vectors", "ort/coreml"]              # macOS: GPU/Neural Engine acceleration
dual-embeddings = ["vectors"]                            # StarEncoder (code) + nomic (text)
mcp = ["tokio"]                                          # Optional: MCP server
full = ["cli", "vectors", "mcp"]

# Platform-specific defaults (set in build.rs or CI)
# - macOS: vectors-coreml
# - Linux/Windows: vectors
# - dual-embeddings: opt-in for code+text hybrid search
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
    TypeScript,
    JavaScript,
    Tsx,
    Jsx,
    Python,
    Go,
    Rust,
    Java,
    C,
    Cpp,
    CSharp,
    Php,
    Ruby,
    Swift,
    Kotlin,
    Liquid,
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
        Language::TypeScript => Ok(tree_sitter_typescript::language_typescript()),
        Language::Tsx => Ok(tree_sitter_typescript::language_tsx()),
        Language::JavaScript => Ok(tree_sitter_javascript::language()),
        Language::Python => Ok(tree_sitter_python::language()),
        Language::Rust => Ok(tree_sitter_rust::language()),
        Language::Go => Ok(tree_sitter_go::language()),
        Language::Java => Ok(tree_sitter_java::language()),
        Language::C => Ok(tree_sitter_c::language()),
        Language::Cpp => Ok(tree_sitter_cpp::language()),
        Language::CSharp => Ok(tree_sitter_c_sharp::language()),
        Language::Php => Ok(tree_sitter_php::language_php()),
        Language::Ruby => Ok(tree_sitter_ruby::language()),
        Language::Swift => Ok(tree_sitter_swift::language()),
        Language::Kotlin => Ok(tree_sitter_kotlin::language()),
        Language::Liquid => Err(ParseError::RegexFallback),
        _ => Err(ParseError::UnsupportedLanguage),
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

Uses [rust-bert](https://github.com/guillaume-be/rust-bert) with ONNX backend for embeddings and [sqlite-vec](https://github.com/asg017/sqlite-vec) for vector search.

On macOS, uses CoreML execution provider for GPU/Neural Engine acceleration.

```rust
use rust_bert::pipelines::sentence_embeddings::{
    SentenceEmbeddingsBuilder, SentenceEmbeddingsModel, SentenceEmbeddingsConfig,
    SentenceEmbeddingsModelType,
};
use rust_bert::resources::LocalResource;
use rusqlite::{ffi::sqlite3_auto_extension, Connection};
use sqlite_vec::sqlite3_vec_init;
use zerocopy::AsBytes;
use std::path::{Path, PathBuf};

#[cfg(target_os = "macos")]
use ort::CoreMLExecutionProvider;

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
    model: SentenceEmbeddingsModel,
    embedding_dim: usize,
}

impl TextEmbedder {
    /// Load model from local path ONLY
    ///
    /// # Security
    /// - Model must be pre-downloaded to ~/.codegraph/models/
    /// - No network requests are made (RemoteResource disabled)
    /// - Model integrity should be verified via checksum
    pub fn load(model_dir: &Path) -> Result<Self, EmbedderError> {
        // Verify model directory exists
        if !model_dir.exists() {
            return Err(EmbedderError::ModelNotFound(model_dir.to_path_buf()));
        }

        // Verify required files exist
        let model_file = model_dir.join("model.onnx");
        let tokenizer_file = model_dir.join("tokenizer.json");
        let config_file = model_dir.join("config.json");

        for file in [&model_file, &tokenizer_file, &config_file] {
            if !file.exists() {
                return Err(EmbedderError::MissingModelFile(file.clone()));
            }
        }

        // Configure for local-only loading (no network)
        let config = SentenceEmbeddingsConfig::new(
            LocalResource::from(model_file),
            LocalResource::from(tokenizer_file),
            LocalResource::from(config_file),
        );

        // Configure execution providers based on platform
        #[cfg(target_os = "macos")]
        let execution_providers = vec![
            CoreMLExecutionProvider::default()
                .with_subgraphs()        // Enable for subgraphs
                .with_ane_only(false)    // Use GPU + Neural Engine + CPU
                .build()
                .error_on_failure(),
        ];

        #[cfg(not(target_os = "macos"))]
        let execution_providers = vec![]; // CPU fallback

        // Build model with ONNX backend
        // On macOS: Uses CoreML (GPU/Neural Engine) with CPU fallback
        // On Linux/Windows: Uses CPU
        let model = SentenceEmbeddingsBuilder::local(config)
            .with_execution_providers(execution_providers)
            .create_model()
            .map_err(|e| EmbedderError::ModelLoadError(e.to_string()))?;

        // Get embedding dimension from model config
        let embedding_dim = 768; // TODO: Read from config.json

        Ok(Self {
            model,
            embedding_dim,
        })
    }

    /// Load a pre-configured model type from local cache
    ///
    /// Models should be pre-downloaded to ~/.codegraph/models/
    pub fn load_model_type(
        model_type: SentenceEmbeddingsModelType,
        cache_dir: &Path,
    ) -> Result<Self, EmbedderError> {
        // Set cache directory to prevent network downloads
        std::env::set_var("RUSTBERT_CACHE", cache_dir);

        let model = SentenceEmbeddingsBuilder::local(model_type)
            .create_model()
            .map_err(|e| EmbedderError::ModelLoadError(e.to_string()))?;

        let embedding_dim = match model_type {
            SentenceEmbeddingsModelType::AllMiniLmL6V2 => 384,
            SentenceEmbeddingsModelType::AllMiniLmL12V2 => 384,
            SentenceEmbeddingsModelType::AllDistilrobertaV1 => 768,
            SentenceEmbeddingsModelType::ParaphraseAlbertSmallV2 => 768,
            _ => 768, // Default
        };

        Ok(Self {
            model,
            embedding_dim,
        })
    }

    /// Generate embedding for text
    pub fn embed(&self, text: &str) -> Result<Vec<f32>, EmbedderError> {
        let embeddings = self.model.encode(&[text])
            .map_err(|e| EmbedderError::InferenceError(e.to_string()))?;

        Ok(embeddings.into_iter().next().unwrap())
    }

    /// Batch embed multiple texts (more efficient than individual calls)
    pub fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbedderError> {
        self.model.encode(texts)
            .map_err(|e| EmbedderError::InferenceError(e.to_string()))
    }

    /// Get the embedding dimension for this model
    pub fn embedding_dim(&self) -> usize {
        self.embedding_dim
    }
}

/// Supported embedding models (pre-downloadable)
#[derive(Debug, Clone, Copy)]
pub enum EmbeddingModel {
    /// all-MiniLM-L6-v2: 384 dimensions, fast, good quality
    AllMiniLmL6V2,
    /// all-MiniLM-L12-v2: 384 dimensions, slightly better quality
    AllMiniLmL12V2,
    /// all-distilroberta-v1: 768 dimensions, best quality
    AllDistilrobertaV1,
}

impl EmbeddingModel {
    pub fn dimension(&self) -> usize {
        match self {
            Self::AllMiniLmL6V2 | Self::AllMiniLmL12V2 => 384,
            Self::AllDistilrobertaV1 => 768,
        }
    }

    pub fn model_id(&self) -> &'static str {
        match self {
            Self::AllMiniLmL6V2 => "sentence-transformers/all-MiniLM-L6-v2",
            Self::AllMiniLmL12V2 => "sentence-transformers/all-MiniLM-L12-v2",
            Self::AllDistilrobertaV1 => "sentence-transformers/all-distilroberta-v1",
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

# Tree-sitter
tree-sitter = "0.22"
tree-sitter-typescript = "0.21"
tree-sitter-javascript = "0.21"
tree-sitter-python = "0.21"
tree-sitter-rust = "0.21"
tree-sitter-go = "0.21"
tree-sitter-java = "0.21"
tree-sitter-c = "0.21"
tree-sitter-cpp = "0.21"
tree-sitter-c-sharp = "0.21"
tree-sitter-php = "0.21"
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

### 9.1 Completeness Issues

| Area | Issue | Mitigation |
|------|-------|------------|
| **Liquid Language** | TypeScript uses regex fallback due to tree-sitter ABI issues. Rust version needs same fallback. | Implement regex-based Liquid extractor in `languages/liquid.rs` |
| **sqlite-vec integration** | sqlite-vec is pre-v1, API may change. | Pin specific version, monitor for breaking changes |
| **ESM Dynamic Import** | TypeScript dynamically imports `@xenova/transformers`. Rust doesn't have equivalent. | Use `rust-bert` with ONNX backend, statically linked |
| **Node Cache (LRU)** | TypeScript QueryBuilder has LRU cache. Plan doesn't mention caching strategy. | Add `lru` crate for node caching in QueryBuilder |
| **Error JSON Arrays** | TypeScript stores errors as JSON arrays in `files.errors`. Plan doesn't specify serialization. | Use `serde_json::to_string(&errors)` for `Vec<String>` |
| **Git Hooks** | TypeScript writes shell scripts. Rust needs to generate platform-appropriate scripts. | Detect platform, generate bash or batch scripts |
| **Memory Monitoring** | TypeScript has memory monitoring utilities. Rust has different memory model. | May not be necessary; Rust has predictable memory usage |
| **vec0 table migrations** | Existing databases won't have vec0 table. | Migration adds vec0 table, re-indexes embeddings if needed |

### 9.2 Accuracy Issues

| Claim | Reality | Correction |
|-------|---------|------------|
| "Same tree-sitter grammars" | Rust tree-sitter crates may have different versions than Node bindings | Pin specific grammar versions; test extraction parity |
| "768-dimensional embeddings" | Dimension depends on model; nomic-embed-text-v1.5 is 768 | Correct, but should be configurable |
| "No network by default" | rust-bert/ort may try to download models or ONNX runtime | Set `RUSTBERT_CACHE` env var, use `ort = { features = ["load-dynamic"] }`, pre-download models |
| "Prepared statements" | rusqlite prepared statements work differently than better-sqlite3 | Use `conn.prepare_cached()` for similar semantics |

### 9.3 Missing Components

1. **Incremental Sync**: Plan mentions sync but doesn't detail change detection algorithm
2. **Progress Callbacks**: TypeScript uses callbacks; Rust needs different pattern (channels or trait)
3. **Concurrent Indexing Prevention**: Need Mutex equivalent
4. **Config Validation**: Schema validation for config.json
5. **Graceful Shutdown**: Signal handling for CLI and MCP server
6. **Cross-Platform Paths**: Windows path handling differs from Unix

### 9.4 Performance Considerations

| TypeScript Behavior | Rust Consideration |
|---------------------|-------------------|
| Single-threaded by default | Can use `rayon` for parallel file processing |
| Async I/O for embeddings | Consider `tokio` for async, but adds complexity |
| V8 GC pauses | No GC, but need careful memory management |
| better-sqlite3 sync API | rusqlite is also sync; good match |

### 9.5 Behavioral Parity Risks

1. **Node ID Generation**: Must produce identical hashes for same inputs
2. **FTS5 Queries**: Tokenization may differ between platforms
3. **File Glob Patterns**: Glob crate may have different semantics
4. **Line Counting**: Off-by-one errors between 0-indexed and 1-indexed
5. **Path Normalization**: Forward vs backward slashes

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
