# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

CodeGraph is a local-first code intelligence system that builds a semantic knowledge graph from any codebase. It provides structural understanding of code relationships using tree-sitter for AST parsing and SQLite for storage.

**Key characteristics:**
- Headless library (no UI) - purely an API
- Rust workspace with 11 crates
- Per-project data stored in `.codegraph/` directory
- Deterministic extraction from AST, not AI-generated summaries
- ONNX embeddings with CoreML acceleration on Apple Silicon

## Build and Development Commands

```bash
# Build (Rust workspace)
cargo build --message-format=json              # Build all crates
cargo check --message-format=json              # Check only (faster)

# Test
cargo test --message-format=json               # Run all tests
cargo test --message-format=json -p codegraph-extraction             # Test specific crate
cargo test --message-format=json -p codegraph-extraction test_name   # Single test

# Lint
cargo clippy --message-format=json             # Run clippy

# Run CLI
cargo run --message-format=json -p codegraph-cli -- index <path>     # Index a codebase
cargo run --message-format=json -p codegraph-cli -- status <path>    # Show statistics

# Verify indexed data
sqlite3 .codegraph/codegraph.db "SELECT name, decorators FROM nodes WHERE decorators != '[]' LIMIT 10;"
```

## Architecture

### Core Module Structure (Rust Workspace)

```
crates/
├── codegraph-types/       # Shared types (Node, Edge, NodeKind, EdgeKind)
├── codegraph-db/          # SQLite with FTS5, schema, prepared statements
├── codegraph-extraction/  # Tree-sitter AST parsing (per-language extractors)
├── codegraph-resolution/  # Reference resolution, framework patterns
├── codegraph-graph/       # BFS/DFS traversal, circular deps, dead code
├── codegraph-vectors/     # ONNX embeddings (ort + CoreML on Apple Silicon)
├── codegraph-context/     # Context building for AI
├── codegraph-sync/        # Incremental updates, git hooks
├── codegraph-mcp/         # MCP server (7 tools)
├── codegraph-core/        # Orchestration layer
└── codegraph-cli/         # CLI entry point
```

### Key Crates

- **codegraph-core**: Main orchestration. Lifecycle methods (`init`, `open`, `close`), indexing, graph queries, semantic search, context building

- **codegraph-extraction**: Coordinates file scanning, parsing, and storing. Tree-sitter grammars for 17 languages (enabled via feature flags)

- **codegraph-graph**: BFS/DFS traversal, call graph construction, impact radius, circular dependency detection, dead code analysis

- **codegraph-vectors**: Manages embeddings using `ort` (ONNX Runtime). CoreML acceleration on Apple Silicon. Stores vectors in SQLite BLOB format

- **codegraph-resolution**: Resolves unresolved references using framework patterns, import resolution, and name matching

### Database Schema

SQLite database with:
- `nodes`: Code symbols (functions, classes, methods, etc.)
- `edges`: Relationships (calls, imports, extends, contains, etc.)
- `files`: Tracked source files with content hashes
- `unresolved_refs`: References pending resolution
- `vectors`: Embeddings stored as BLOBs
- `nodes_fts`: FTS5 virtual table for full-text search

### Supported Languages

TypeScript, JavaScript, Python, Go, Rust, Java, C, C++, C#, PHP, Ruby, Swift, Kotlin, Dart, GraphQL, Bash, HCL (Terraform)

**Feature flags** (in `codegraph-extraction`):
- Default: `lang-rust`, `lang-typescript`, `lang-python`, `lang-go`, `lang-php`
- Enable all: `cargo build --message-format=json --features all-languages`
- Enable specific: `cargo build --message-format=json --features lang-dart,lang-graphql`

### Node and Edge Types

**NodeKind**: `file`, `module`, `class`, `struct`, `interface`, `trait`, `protocol`, `function`, `method`, `property`, `field`, `variable`, `constant`, `enum`, `enum_member`, `type_alias`, `namespace`, `parameter`, `import`, `export`, `route`, `component`

**EdgeKind**: `contains`, `calls`, `imports`, `exports`, `extends`, `implements`, `references`, `type_of`, `returns`, `instantiates`, `overrides`, `decorates`

## CLI Usage

```bash
# During development (use cargo run)
cargo run --message-format=json -p codegraph-cli -- init <path>
cargo run --message-format=json -p codegraph-cli -- index <path>

# If installed globally
codegraph init [path]       # Initialize in project
codegraph index [path]      # Full index
codegraph sync [path]       # Incremental update
codegraph status [path]     # Show statistics
codegraph query <search>    # Search symbols
codegraph context <task>    # Build context for AI
codegraph hooks install     # Install git auto-sync
codegraph serve --mcp       # Start MCP server
```

## MCP Tools Best Practices

These tools are designed to be used by **Explore agents** for faster codebase exploration:

| Tool | Use For |
|------|---------|
| `codegraph_search` | Find symbols by name (functions, classes, types) |
| `codegraph_context` | Get relevant code context for a task |
| `codegraph_callers` | Find what calls a function |
| `codegraph_callees` | Find what a function calls |
| `codegraph_impact` | See what's affected by changing a symbol |
| `codegraph_node` | Get details + source code for a symbol |

### Important
CodeGraph provides **code context**, not product requirements. For new features, still ask the user about:
- UX preferences and behavior
- Edge cases and error handling
- Acceptance criteria

## Test Structure

Each crate has tests in `src/` (unit tests) and `tests/` (integration tests):
- `codegraph-db` - Database, schema, prepared statements
- `codegraph-extraction` - Tree-sitter parsing for all languages
- `codegraph-resolution` - Reference resolution
- `codegraph-graph` - Traversal and graph queries
- `codegraph-vectors` - Embedding and semantic search
- `codegraph-context` - Context building
- `codegraph-sync` - Incremental updates and git hooks

Tests use temporary directories created with `tempfile` crate and cleaned up after each test.

## Known Issues / In Progress

**Tree-sitter wiring (P0):** Tree-sitter grammars are declared in `Cargo.toml` but extraction currently uses regex. This prevents decorator/attribute extraction. See `docs/plans/LINKED_REPOS.md` and the deviation audit plan.

**Key plan documents:**
- `RUST_REWRITE_PLAN.md` - Original requirements
- `docs/plans/LINKED_REPOS.md` - Multi-repo feature design

## Verifying Tree-sitter Grammar Support

To check if a tree-sitter grammar supports specific AST nodes:
```bash
# Fetch grammar.js and search for node types
curl -s https://raw.githubusercontent.com/tree-sitter/tree-sitter-typescript/master/common/define-grammar.js | grep -A5 "decorator:"
curl -s https://raw.githubusercontent.com/tree-sitter/tree-sitter-rust/master/grammar.js | grep -A5 "attribute_item:"
```
