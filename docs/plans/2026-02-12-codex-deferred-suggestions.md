# Deferred Codex Suggestions — Incremental Embedding Sync

Suggestions from Codex code reviews during implementation that were acknowledged
but not acted on. These are valid observations worth revisiting in future work.

## Tasks 2-5 Review

### O(k) queries per node in get_embedding_neighbors/siblings
**Severity:** Low
**Context:** `get_embedding_neighbors()` issues separate SQL queries per input node.
For large change sets this could be slow.
**Why deferred:** Acceptable for typical sync sizes (1-20 files). The design already
handles the worst case (full re-embed fallback via `should_full_reembed()`).
**Future fix:** Batch queries with `IN` clauses if profiling shows bottlenecks.

### SQLite variable limit in delete_batch
**Severity:** Low
**Context:** `VectorStorage::delete_batch()` builds an `IN (?)` clause with one
placeholder per node ID. SQLite has a default limit of 999 variables.
**Why deferred:** Typical deletes are small. If a sync deletes >999 nodes, the
design falls back to full re-embed. Could chunk into batches of 500 if needed.

## Tasks 7-9 Review

### Pre-sync edge snapshot races with concurrent syncs (Finding 1)
**Severity:** Medium (theoretical)
**Context:** `EdgeSnapshot::capture()` happens before `SyncManager` acquires the
`IndexLock`. In theory, another process could modify edges between snapshot and lock.
**Why deferred:** `CodeGraph::sync()` takes `&mut self` (Rust borrow checker prevents
same-process concurrency). Cross-process races are low-risk since the lock is
acquired immediately after. Real-world sync is single-process.
**Future fix:** Move snapshot capture inside `SyncManager` under the lock, or add
a two-phase sync API.

### Full-table EdgeSnapshot is O(edges) every sync (Finding 4)
**Severity:** Medium (performance)
**Context:** `EdgeSnapshot::capture(self.db.conn())` reads ALL edges before every
sync, even when there are no changes. For large graphs (100k+ edges) this adds
latency.
**Why deferred:** For typical projects (10k-50k edges), this is <50ms. The
optimization (detect changes first, then scope snapshot to changed files) requires
refactoring `SyncManager` to separate detection from processing.
**Future fix:** Add `SyncManager::detect_changes()` public method, then use
`EdgeSnapshot::capture_for_files()` with only the changed file paths.

## Task 10 Review

### Public API break: sync() return type changed (Finding 1)
**Severity:** High (for external consumers)
**Context:** `CodeGraph::sync()` now returns `FullSyncResult` instead of
`codegraph_sync::SyncResult`. Breaking change for any downstream callers.
**Why deferred:** Pre-1.0 library with only the CLI as consumer (already updated).
No external dependents exist yet. When stabilizing the API, consider adding
`sync_with_embeddings()` separately or `impl From<FullSyncResult> for SyncResult`.

### Missing tests for full_reembed/skipped_no_model combinations (Finding 3)
**Severity:** Low
**Context:** Integration tests cover sync flow but can't test embedding path because
ONNX model isn't available in test environment. The `full_reembed` + `skipped_no_model`
interaction is only testable with a mocked embedder.
**Why deferred:** Would require creating a mock/test embedder infrastructure. The logic
paths are simple conditionals. Real embedding behavior is tested manually via CLI.
