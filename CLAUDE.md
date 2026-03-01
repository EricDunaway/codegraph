# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

CodeGraph is a local-first code intelligence system that builds a semantic knowledge graph from any codebase. It provides structural understanding of code relationships using tree-sitter for AST parsing and SQLite for storage.

**Key characteristics:**
- Headless library (no UI) - purely an API
- Rust workspace with 12 crates
- Per-project data stored in `.codegraph/` directory
- Two-stage extraction: tree-sitter AST parsing creates nodes + unresolved references, then resolution converts refs to edges
- ONNX embeddings with CoreML acceleration on Apple Silicon
- LSP integration for type enrichment (TypeScript, Rust, Python, Go, Dart)

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
├── codegraph-types/       # Shared types (Node, Edge, NodeKind, EdgeKind, Language)
├── codegraph-db/          # SQLite with FTS5, schema, prepared statements, enrichment_deps
├── codegraph-extraction/  # Tree-sitter AST parsing, code snippets, test detection, package detection
├── codegraph-resolution/  # Reference resolution (import, name matching, framework patterns)
├── codegraph-graph/       # BFS/DFS traversal, call graphs, impact radius, circular deps, dead code
├── codegraph-vectors/     # ONNX embeddings (ort + CoreML), enriched text building, token budgeting
├── codegraph-context/     # Context building for AI (FTS + semantic strategies)
├── codegraph-sync/        # Incremental updates, git diff detection, edge diffing, checkpoints, locking
├── codegraph-lsp/         # LSP type enrichment (TypeScript, Rust, Python, Go, Dart)
├── codegraph-mcp/         # MCP server (8 tools)
├── codegraph-core/        # Orchestration layer (45+ public methods)
└── codegraph-cli/         # CLI entry point
```

Each complex crate has its own CLAUDE.md with detailed API docs. See `crates/<name>/CLAUDE.md`.

### Extraction Pipeline (two-stage)

1. **Extraction** (`codegraph-extraction`): Tree-sitter parses AST → creates nodes + `UnresolvedReference` records with `reference_kind` (Calls, Imports, Extends). Also creates structural `contains` edges. Extracts code snippets, decorators, test associations.
2. **Resolution** (`codegraph-resolution`): Converts `UnresolvedReference` → actual `Edge` records using import resolution, name matching, and framework patterns. Creates `calls`, `imports`, `extends` edges.

### Key Crates (see per-crate CLAUDE.md for details)

- **codegraph-core**: Main orchestration. 45+ public methods: lifecycle (`init`, `open`), indexing, graph queries (`get_callers`, `get_callees`, `get_impact_radius`, `find_circular_dependencies`, `find_dead_code`), semantic search, context building
- **codegraph-extraction**: File scanning, tree-sitter parsing for 17 languages, code snippet extraction, test detection, package/module detection, error extraction
- **codegraph-graph**: BFS/DFS traversal, call graph construction, impact radius, circular dependency detection, dead code analysis, node metrics, type hierarchy, embedding neighbor queries
- **codegraph-vectors**: ONNX embeddings with CoreML acceleration, enriched text building (includes graph neighbors), token budgeting, vector storage in SQLite BLOB
- **codegraph-resolution**: Resolves unresolved references → edges using import resolution, name matching, framework patterns, scoped resolution (`resolve_for_files`)
- **codegraph-sync**: Git diff-based change detection, edge diffing (`EdgeSnapshot`/`EdgeDiff`), checkpoint system, pending sync coalescing, PID-verified locking, selective enrichment scope, git hook management (post-commit/checkout/merge/rewrite)
- **codegraph-lsp**: LSP type enrichment for TypeScript, Rust, Python, Go, Dart — inferred types, hover data, definition locations

### Database Schema

SQLite database with:
- `nodes`: Code symbols (functions, classes, methods, etc.) with code_snippet, decorators, visibility
- `edges`: Relationships (calls, imports, extends, contains, etc.) with unique index for deduplication
- `files`: Tracked source files with content hashes
- `unresolved_refs`: References pending resolution (with `resolved` column for tracking)
- `vectors`: Embeddings stored as BLOBs
- `nodes_fts`: FTS5 virtual table for full-text search
- `enrichment_deps`: Dependencies between enrichment operations (node→file tracking)
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

## MCP Tools

8 tools for IDE/agent integration via JSON-RPC stdio:

| Tool | Use For |
|------|---------|
| `codegraph_context` | **PRIMARY** — Comprehensive code context for a task (semantic + FTS) |
| `codegraph_search` | Find symbols by name — returns node IDs for follow-up tools |
| `codegraph_node` | Get symbol details with code snippet, signature, docstring |
| `codegraph_file_nodes` | List all symbols in a file |
| `codegraph_callers` | Find callers of a function (requires resolved edges) |
| `codegraph_callees` | Find callees of a function (requires resolved edges) |
| `codegraph_impact` | Blast radius analysis (direct + indirect dependents) |
| `codegraph_status` | Index health, dirty files, hook status |

**Workflow:** Start with `codegraph_context` — it's often sufficient alone. Use `codegraph_search` to find node IDs, then `codegraph_callers`/`codegraph_callees` to trace call chains. Use `codegraph_impact` before changes.

**Note:** `codegraph_callers`, `codegraph_callees`, and `codegraph_impact` require resolved edges (full `codegraph index` run). Tools show staleness warnings if files are dirty.

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

**Signatures/docstrings not populated:** `nodes.signature` and `nodes.docstring` columns are NULL for most nodes. Code snippets ARE populated.

**Plan documents:**

Active:
- `docs/plans/2026-02-04-embedding-enrichment-design.md` - Embedding enrichment (PARTIALLY IMPLEMENTED — LSP crate + enrichment_deps done)
- `docs/plans/2026-02-23-daemon-branch-db.md` - Daemon + per-branch DB (FUTURE — sync prerequisites met)
- `docs/plans/CROSS_LANGUAGE_LINKING.md` - Cross-language/repo linking (FUTURE)

Completed (archived in `docs/plans/completed/`):
- Sync redesign design + implementation — IMPLEMENTED 2026-02-28
- LINKED_REPOS.md — SUPERSEDED by CROSS_LANGUAGE_LINKING.md

Reference: `RUST_REWRITE_PLAN.md` (original requirements)

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
- **Plan doc cleanup after implementation** - when completing a plan task, mark it done in the plan doc. When all tasks in a plan are complete, add `**Status: IMPLEMENTED (date)**` at the top. Superseded plans get `**Status: SUPERSEDED by [link]**`
- **Per-crate CLAUDE.md maintenance** - when adding/changing public API in a crate, update its `crates/<name>/CLAUDE.md`
