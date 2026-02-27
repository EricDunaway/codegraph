# Changelog

All notable changes to CodeGraph will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Auto-add `.codegraph/` to `.gitignore` on project init (when inside a git repo)

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
