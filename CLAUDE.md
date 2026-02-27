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
sqlite3 .codegraph/codegraph.db "SELECT name, decorators FROM nodes WHERE decorators <> '[]' LIMIT 10;"

# Verify extraction state
sqlite3 .codegraph/codegraph.db "SELECT kind, COUNT(*) FROM edges GROUP BY kind;"  # Should show calls, imports, not just contains
sqlite3 .codegraph/codegraph.db "SELECT COUNT(*) FROM nodes WHERE code_snippet IS NOT NULL;"  # Should be >0
sqlite3 .codegraph/codegraph.db "SELECT name, decorators FROM nodes WHERE length(decorators) > 2 LIMIT 5;"  # Verify decorators work
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

- **codegraph-sync**: Incremental file change detection, edge diff computation (`EdgeSnapshot`/`EdgeDiff`), full re-embed trigger detection (`reembed.rs`), selective scope for cascade enrichment, file locking

### Database Schema

SQLite database with:
- `nodes`: Code symbols (functions, classes, methods, etc.)
- `edges`: Relationships (calls, imports, extends, contains, etc.)
- `files`: Tracked source files with content hashes
- `unresolved_refs`: References pending resolution
- `vectors`: Embeddings stored as BLOBs
- `nodes_fts`: FTS5 virtual table for full-text search
- `metadata`: Key-value store for schema version, embedding config hash, model hash
- `schema_version`: Migration tracking (version, applied_at, description)

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

| Tool | Status | Use For |
|------|--------|---------|
| `codegraph_search` | ✅ | Find symbols by name (functions, classes, types) |
| `codegraph_context` | ✅ | Get relevant code context for a task |
| `codegraph_file_nodes` | ✅ | List all symbols in a file |
| `codegraph_status` | ✅ | Index status and dirty file detection |
| `codegraph_node` | ⚠️ | Get symbol location (code snippet not yet implemented) |
| `codegraph_callers` | ❌ | Find what calls a function (needs edge extraction) |
| `codegraph_callees` | ❌ | Find what a function calls (needs edge extraction) |
| `codegraph_impact` | ❌ | See what's affected by changes (needs edge extraction) |

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

**See also:** `docs/issues.md` (bugs) and `docs/gaps.md` (feature gaps)

**Gotcha:** `!=` gets escaped to `\!=` in this environment. Use `<>` or `length(column) > 2` instead.

**Gotcha:** `edges` table CASCADE deletes when `nodes` are deleted. Capture `EdgeSnapshot` BEFORE `SyncManager::sync()` if you need pre-sync edge state.

**Gotcha:** `TextEmbedder::load()` creates an ONNX session (~100ms). Avoid loading twice in the same code path. `generate_embeddings()` returns `Ok(0)` for both "model not found" and "no embeddable nodes" — check model availability separately if the distinction matters.

**Relationship edges not implemented (P0):** Tree-sitter extraction works and captures decorators (verified), but only creates `contains` edges (structural). No `calls`, `imports`, or `extends` edges are created, which breaks `codegraph_callers`, `codegraph_callees`, and `codegraph_impact` tools. See `crates/codegraph-extraction/src/tree_sitter_extractor.rs` lines 124-129.

**Key plan documents:**
- `RUST_REWRITE_PLAN.md` - Original requirements
- `docs/plans/LINKED_REPOS.md` - Multi-repo feature design
- `docs/plans/2026-02-12-incremental-embedding-sync-design.md` - Incremental embedding sync design
- `docs/plans/2026-02-12-incremental-embedding-sync-impl.md` - Implementation plan (11 tasks)

## Verifying Tree-sitter Grammar Support

To check if a tree-sitter grammar supports specific AST nodes:
```bash
# Fetch grammar.js and search for node types
curl -s https://raw.githubusercontent.com/tree-sitter/tree-sitter-typescript/master/common/define-grammar.js | grep -A5 "decorator:"
curl -s https://raw.githubusercontent.com/tree-sitter/tree-sitter-rust/master/grammar.js | grep -A5 "attribute_item:"
```

## Versioning & Changelog

**Single source of truth:** `CHANGELOG.md` (Keep a Changelog format)
**Version location:** `Cargo.toml` → `[workspace.package] version`
**Runtime access:** `env!("CARGO_PKG_VERSION")` (used in MCP server response)

### When to update
- **New feature**: Add entry under `## [Unreleased]` → `### Added`
- **Bug fix**: Add entry under `## [Unreleased]` → `### Fixed`
- **Breaking change**: Add entry under `## [Unreleased]` → `### Changed` or `### Removed`
- **Deprecation**: Add entry under `## [Unreleased]` → `### Deprecated`
- **Refactor with no behavior change**: No changelog entry needed

### How to update
1. Add a concise line to the appropriate section in `CHANGELOG.md` under `## [Unreleased]`
2. When cutting a release, move `[Unreleased]` entries to a new `## [X.Y.Z] - YYYY-MM-DD` section
3. Update `Cargo.toml` workspace version to match: `[workspace.package] version = "X.Y.Z"`

### Skills/agents MUST
- Add a changelog entry when completing any feature, fix, or behavioral change
- Keep entries user-facing and concise (what changed, not how)
- Never modify existing released version entries — only add to `[Unreleased]`

## Documentation Maintenance

- **Always verify claims against code** - don't trust plan docs; check actual implementation
- **Use `docs/issues.md`** for bugs, **`docs/gaps.md`** for missing features
- **Don't track fixed issues** - remove from issues.md once resolved
