# Changelog

All notable changes to CodeGraph will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- ImpactCapture: targeted pre-delete neighbor queries replacing O(E) EdgeSnapshot for embed candidate computation
- Scoped reference resolution via `resolve_for_files()` (two-sided source + target) instead of global `resolve_all()`
- Git diff-based change detection (`git diff --name-status`) with rename/copy handling, fallback to hash scan
- Checkpoint system (`sync.last_head`) for tracking last-synced git HEAD
- Pending sync coalescing (`sync.pending`) for concurrent hook events
- `--hook` and `--verify-sync` CLI flags for sync command
- `--force` flag for hooks install to bypass hook manager detection
- `--verify-sync` EdgeSnapshot verification mode for testing ImpactCapture coverage
- Git hooks: post-rewrite support, Husky/Lefthook detection, hook chaining via `.codegraph-orig` backup
- Auto-add `.codegraph/` to `.gitignore` on project init and hooks install
- `Component` added to embeddable node kinds
- Schema v3 migration: edge deduplication (unique index), `resolved` column on unresolved_refs

### Changed
- Sync pipeline redesigned as 7-phase flow: Trigger → Detect → Lock → Extract → Resolve → Embed → Release/Drain
- `index_all()` now acquires IndexLock and clears all tables before rebuild
- `process_modify()` now parses files before deleting old nodes (prevents data loss)
- Edge inserts use `INSERT OR IGNORE` to prevent duplicates
- Lock system: PID ownership verification, heartbeat refresh at phase boundaries, `try_acquire_or_pending()` for hooks
- Hook scripts rewritten with exit code preservation and background sync execution
- Full-reindex fallback requires minimum 10 tracked files (prevents triggering on small projects)

### Fixed
- `process_modify()` parse-before-delete prevents data loss on extraction failure
- Edge insert failures no longer silently mark references as resolved
- `--verify-sync` correctly excludes deleted nodes from EdgeDiff comparison
- `ModelLoadFailed` handled gracefully in embedding generation (CoreML compilation failures)

## [0.1.0] - 2026-02-26

### Added
- Rust workspace with 11 crates replacing TypeScript implementation
- Tree-sitter AST extraction for 17 languages with feature flags
- SQLite database with FTS5 full-text search and schema migration system (v1-v3)
- Node and edge model: 22 node kinds, 12 edge kinds
- Reference resolution with framework-aware patterns
- Graph traversal: BFS/DFS, call graphs, impact radius, circular dependency detection, dead code analysis
- ONNX embeddings with CoreML acceleration on Apple Silicon
- Vector storage in SQLite BLOB format with semantic search
- Context building for AI consumption
- Incremental sync with file change detection and edge diff computation
- MCP server with 7 tools (search, context, file_nodes, status, node, callers, callees)
- CLI: init, index, sync, status, query, context, hooks, serve
- Per-project data stored in `.codegraph/` directory
