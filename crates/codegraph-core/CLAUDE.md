# codegraph-core

Orchestration crate that provides the single public API surface (`CodeGraph` struct) for the entire system. Wires together extraction, resolution, graph traversal, context building, sync, and embeddings into a coherent interface. All other crates are implementation details; consumers only need this crate.

## Dependencies

Depends on all workspace crates: `codegraph-types`, `codegraph-db`, `codegraph-extraction`, `codegraph-resolution`, `codegraph-graph`, `codegraph-vectors`, `codegraph-context`, `codegraph-sync`.

**Feature flag:** `onnx` -- enables ONNX embedding support (forwards to `codegraph-vectors/onnx`).

## CodeGraph Struct -- Public API

### Lifecycle

| Method | Signature | Notes |
|--------|-----------|-------|
| `init` | `(path) -> Result<Self>` | Creates `.codegraph/` dir + DB. Auto-adds `.codegraph/` to `.gitignore`. |
| `open` | `(path) -> Result<Self>` | Opens existing project. Errors with `NotInitialized` if `.codegraph/` missing. |
| `in_memory` | `() -> Result<Self>` | In-memory DB for tests. Root is `.` (no real project). |
| `with_config` | `(CodeGraphConfig) -> Result<Self>` | Custom config. Creates data dir if needed. |

### Indexing

| Method | Signature | Notes |
|--------|-----------|-------|
| `index_all` | `(&mut self) -> Result<IndexingResult>` | Full reindex: scan, extract, resolve refs, generate embeddings. Clears all graph data first. Acquires `IndexLock`. |
| `index_all_dry_run` | `(&self) -> Result<ExtractionIndexResult>` | Extract without storing. Read-only. |
| `sync` | `(&mut self) -> Result<FullSyncResult>` | Incremental sync with default options. |
| `sync_with_options` | `(&mut self, SyncOptions) -> Result<FullSyncResult>` | Full 7-phase pipeline: trigger, detect (git diff / hash scan / file list), lock, extract, resolve, embed, checkpoint, release + drain pending. Falls back to full reindex if >30% files changed. |

### Search

| Method | Signature | Notes |
|--------|-----------|-------|
| `search` | `(&self, query, limit) -> Result<Vec<SearchResult>>` | FTS5 search by symbol name. |
| `search_by_kind` | `(&mut self, query, NodeKind, limit) -> Result<Vec<SearchResult>>` | FTS5 with kind filter. |
| `get_node` | `(&mut self, node_id) -> Result<Option<Node>>` | Lookup by ID. |
| `get_nodes_in_file` | `(&self, file_path) -> Result<Vec<Node>>` | All nodes in a file. |
| `semantic_search` | `(&self, query, limit) -> Result<Option<Vec<SimilarityResult>>>` | Embeds query with `search_query:` prefix, cosine similarity. Returns `None` if embeddings unavailable (no model/vectors). Min score threshold: 0.3. |
| `search_by_vector` | `(&self, query_vector, limit) -> Result<Vec<SimilarityResult>>` | Manual vector search without loading the embedder. |

### Graph Queries

| Method | Signature | Notes |
|--------|-----------|-------|
| `get_callers` | `(&mut self, node_id) -> Result<Vec<Node>>` | Nodes that call this node. Requires resolved edges. |
| `get_callees` | `(&mut self, node_id) -> Result<Vec<Node>>` | Nodes this node calls. Requires resolved edges. |
| `get_call_graph` | `(&mut self, node_id) -> Result<CallGraph>` | Bidirectional call graph via `GraphQueryManager`. |
| `get_impact_radius` | `(&mut self, node_id, max_depth) -> Result<ImpactRadius>` | Transitive dependents up to `max_depth` hops. |

### Context Building

| Method | Signature | Notes |
|--------|-----------|-------|
| `build_context` | `(&mut self, query) -> Result<ContextResult>` | FTS-based context with default options. |
| `build_context_with_options` | `(&mut self, query, ContextOptions) -> Result<ContextResult>` | FTS-based with custom options. |
| `build_context_for_node` | `(&mut self, node_id, query) -> Result<ContextResult>` | Context centered on a specific node. |
| `build_context_semantic` | `(&mut self, query) -> Result<ContextResult>` | Semantic search first, FTS fallback. |
| `build_context_semantic_with_options` | `(&mut self, query, ContextOptions) -> Result<ContextResult>` | Semantic + FTS with custom options. |

### Embeddings

| Method | Signature | Notes |
|--------|-----------|-------|
| `store_embedding` | `(&self, node_id, &[f32], model) -> Result<()>` | Store a pre-computed embedding. |
| `sync_embeddings` | `(&mut self, SyncResult, EdgeDiff) -> Result<EmbeddingSyncResult>` | Legacy EdgeDiff-based incremental embedding sync. Retained for `--verify-sync`. |

**Embeddable node kinds** (const `EMBEDDABLE_KINDS`): Function, Method, Class, Struct, Interface, Trait, Enum, Module, Component.

### Statistics / Accessors

| Method | Signature | Notes |
|--------|-----------|-------|
| `get_stats` | `(&self) -> Result<ProjectStats>` | Node, edge, and file counts. |
| `config` | `(&self) -> &CodeGraphConfig` | Read-only config. |
| `conn` | `(&self) -> &Connection` | Raw SQLite connection for advanced queries. |
| `queries` / `queries_mut` | `-> &QueryBuilder` / `-> &mut QueryBuilder` | Prepared statement access. |
| `is_initialized` | `(&self) -> bool` | Always true after successful construction. |
| `root` | `(&self) -> &Path` | Project root directory. |

## Key Types

- **`SyncOptions`** -- Controls sync behavior: `hook_name` (enables hook-mode with non-blocking lock), `file_list` (override detection), `verify_sync` (EdgeSnapshot comparison).
- **`IndexingResult`** -- `files_indexed`, `nodes_created`, `edges_created`, `references_resolved`, `embeddings_generated`.
- **`FullSyncResult`** -- Wraps `codegraph_sync::SyncResult` + `EmbeddingSyncResult`.
- **`EmbeddingSyncResult`** -- `vectors_deleted`, `vectors_created`, `vectors_updated`, `skipped_no_model`, `full_reembed`.
- **`ProjectStats`** -- `node_count`, `edge_count`, `file_count`.
- **`CodeGraphConfig`** -- Root, data dir, DB path, excludes, max file size, LSP/enrichment/embedding configs. Builder pattern with `with_*` methods. Loads from `.codegraph/config.json` via `CodeGraphConfig::load()`.
- **`CodeGraphError`** -- Unified error enum wrapping all sub-crate errors plus `NotInitialized`, `AlreadyInitialized`, `InvalidPath`, `NodeNotFound`, `Config`, `Embedding`, `Other`.

## Helper Functions (public)

| Function | Location | Notes |
|----------|----------|-------|
| `build_embedding_text` | `embedding.rs` | Build embedding text for a node with full graph context (callers, callees, siblings, inheritance). |
| `build_embedding_text_with_budget` | `embedding.rs` | Same but enforces token budget via `TokenCounter`. |

## Re-exports

- All types from `codegraph_types` (wildcard)
- Sub-crates for advanced usage: `context`, `db`, `extraction`, `graph`, `resolution`, `sync`, `vectors`
- Config: `CodeGraphConfig`, `DEFAULT_EXCLUDE_PATTERNS`

## Common Usage Patterns

```rust
// Full index
let mut cg = CodeGraph::init("/path/to/project")?;
let result = cg.index_all()?;

// Open + incremental sync
let mut cg = CodeGraph::open("/path/to/project")?;
let result = cg.sync()?;

// Hook-triggered sync (non-blocking)
let result = cg.sync_with_options(SyncOptions {
    hook_name: Some("post-commit".into()),
    ..Default::default()
})?;

// Search + context
let hits = cg.search("myFunction", 10)?;
let ctx = cg.build_context_semantic("implement caching")?;

// Graph traversal
let callers = cg.get_callers("node_id_here")?;
let impact = cg.get_impact_radius("node_id_here", 3)?;

// Advanced: raw DB access
let conn = cg.conn();
let queries = cg.queries();
```

## Gotchas

- **`index_all` clears everything first.** It calls `clear_all_graph_data()` before re-extracting. Do not call it to "add" files -- use `sync()` for incremental updates.
- **`generate_embeddings` returns `Ok(0)` ambiguously.** Both "model not found" and "no embeddable nodes" return 0. Check model availability separately if the distinction matters.
- **`semantic_search` returns `Option`.** `None` = embeddings unavailable (no model or no vectors). `Some(empty_vec)` = embeddings exist but nothing matched above 0.3 threshold.
- **Sync >30% fallback.** If sync detects more than 30% of tracked files changed (and at least 10 files tracked), it drops down to a full `index_all` reindex.
- **Lock contention in hook mode.** `sync_with_options` with a `hook_name` uses `try_acquire_or_pending` -- if the lock is held, it writes `sync.pending` and returns early (no error). The lock holder drains pending events in Phase 7.
- **`search` vs `search_by_kind` receiver.** `search` takes `&self`, `search_by_kind` takes `&mut self`. Inconsistency to be aware of when borrowing.
- **`get_callers`/`get_callees` require resolved edges.** If you skip resolution (`config.resolve_references = false`) or only have `contains` edges, these return empty results.
- **`TextEmbedder::load()` is ~100ms.** Avoid loading twice in the same code path. `semantic_search` and `sync` each load their own instance internally.
