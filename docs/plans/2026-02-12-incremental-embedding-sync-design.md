# Incremental Embedding Sync

**Date:** 2026-02-12
**Status:** Design
**Crates affected:** `codegraph-core`, `codegraph-sync`, `codegraph-graph`, `codegraph-vectors`

## Problem

`codegraph sync` detects changed files and incrementally re-extracts nodes/edges, but does not update embeddings. The only way to update embeddings is `codegraph index`, which clears ALL vectors and re-embeds every node from scratch. This means:

1. **New/modified files** get nodes but no embeddings — semantic search misses them
2. **Deleted files** leave orphaned vectors in the DB — search returns stale results
3. **Ripple effects** are ignored — when node X is removed, nodes whose embedding text referenced X (callers, callees, siblings) have stale embeddings

## Design Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Where does embedding logic live? | `CodeGraph::sync()` in `codegraph-core` | Matches existing architecture; `generate_embeddings` already lives here |
| Ripple precision | Over-approximate (all direct neighbors) | Simpler, safer, guarantees correctness |
| Cascading re-embeddings? | No — 1 hop only | Embedding text includes node **names**, not other embeddings' content |
| Impact radius reuse? | No — new `get_embedding_neighbors()` | Impact radius traverses wrong edge types (see analysis below) |
| Inline vs deferred? | Inline during sync | Keep it simple; CoreML makes embedding fast; deferred can come later |
| Full vs incremental decision | Use existing `reembed.rs` | `should_full_reembed()` already detects schema/config/model changes |

## Why Impact Radius Cannot Be Reused

`get_impact_radius()` in `codegraph-graph/src/queries.rs` performs BFS over **incoming** `Calls`/`References`/`Imports` edges only. For embedding staleness we need:

| What embedding text includes | Edge traversal needed | Impact radius covers? |
|------------------------------|----------------------|----------------------|
| `calls: X, Y` (callees) | Outgoing `Calls` from node | No — only walks incoming |
| `called by: A, B` (callers) | Incoming `Calls` to node | Yes |
| `siblings: M, N` | `Contains` edges (same parent) | No — not traversed |
| `extends: Base` | Outgoing `Extends` | No — not traversed |
| `implements: Trait` | Incoming `Implements` | No — not traversed |

Impact radius is also **transitive** (multi-hop BFS), while embedding context is **direct-only**. It would over-expand the affected set. A dedicated 1-hop function aligned with `build_graph_context()` is needed.

### Cascading Analysis

**Question:** If re-embedding node A changes its text (e.g., removes callee X), and node B lists A as a sibling, does B need re-embedding too?

**Answer: No.** `EmbeddingTextBuilder::build_text()` only includes **names/identifiers** from `GraphContext` — never another node's embedding content. Node B's embedding says `siblings: A, C` — A's name hasn't changed, only A's embedding content changed. B's text is still accurate.

**Edge case:** `sort_nodes_by_priority` in `GraphTraverser` uses global call frequency for ordering. Removing X could change call counts, reordering truncated lists in unrelated nodes. We accept this as an approximation — the over-approximate neighbor set already handles the vast majority of staleness. Precise ordering-aware re-embedding can be added later if needed.

## Existing Infrastructure

The codebase already has building blocks that are **built but not wired up**:

| Component | File | What it does |
|-----------|------|-------------|
| `EdgeSnapshot` | `codegraph-sync/src/edge_diff.rs` | Captures all edges at a point in time; supports `capture_for_files()` |
| `EdgeDiff` | `codegraph-sync/src/edge_diff.rs` | Computes added/removed edges and `affected_nodes` between two snapshots |
| `should_full_reembed()` | `codegraph-sync/src/reembed.rs` | Detects schema/config/model changes requiring full re-embed |
| `record_embed_metadata()` | `codegraph-sync/src/reembed.rs` | Records embedding metadata after successful embed |
| `VectorStorage::delete()` | `codegraph-vectors/src/storage.rs` | Deletes a single vector by node ID |
| `VectorStorage::store()` | `codegraph-vectors/src/storage.rs` | Upserts a vector (INSERT ON CONFLICT UPDATE) |
| `build_embedding_text()` | `codegraph-core/src/embedding.rs` | Builds rich embedding text with graph context for a single node |
| `SelectiveScope` | `codegraph-sync/src/selective.rs` | Tracks which files/nodes need re-enrichment (can extend for embedding) |

## Data Flow

```
CodeGraph::sync()
│
├─ 1. Check should_full_reembed()
│     If true → full re-embed (clear + rebuild all) → return
│
├─ 2. EdgeSnapshot::capture_for_files(changed_files)     ← PRE-DELETION snapshot
│
├─ 3. SyncManager::sync()                                ← processes file changes
│     ├─ process_add()    → new nodes + edges
│     ├─ process_modify() → delete old nodes, insert new nodes + edges
│     └─ process_delete() → delete nodes (CASCADE deletes edges)
│     Returns: SyncResult { changed_files, enrichment_scope }
│
├─ 4. Reference resolution                               ← finalize graph state
│
├─ 5. EdgeSnapshot::capture_for_files(changed_files)     ← POST-DELETION snapshot
│
├─ 6. EdgeDiff::compute(before, after)                   ← affected_nodes from edge changes
│
├─ 7. Compute embedding update sets:
│     ├─ deleted_node_ids  = old nodes not in new snapshot
│     ├─ new_node_ids      = new nodes from added/modified files
│     ├─ ripple_node_ids   = EdgeDiff.affected_nodes \ deleted_node_ids \ new_node_ids
│     ├─ sibling_node_ids  = get_embedding_siblings(changed_node_ids)  ← Contains edges
│     └─ embed_candidates  = new_node_ids ∪ ripple_node_ids ∪ sibling_node_ids
│
├─ 8. Delete stale vectors:
│     └─ VectorStorage::delete() for each deleted_node_id
│
├─ 9. Generate embeddings for embed_candidates:
│     ├─ Filter to EMBEDDABLE_KINDS only
│     ├─ build_embedding_text() for each node
│     ├─ embedder.embed_document() for each text
│     └─ VectorStorage::store() for each embedding (upserts)
│
└─ 10. record_embed_metadata()
```

### Why Pre-Deletion Snapshot is Critical

`SyncManager::process_modify()` deletes old nodes before inserting new ones. Edges use `ON DELETE CASCADE`, so old edges disappear. Without a pre-deletion snapshot:

- If `funcA` called deleted `funcX`, we'd never know `funcA` needs re-embedding
- If `ClassB` extended deleted `ClassX`, we'd miss `ClassB`'s stale embedding
- Siblings of deleted nodes would retain ghost references

The `EdgeSnapshot::capture_for_files()` before sync captures these relationships. The post-sync snapshot captures new relationships. `EdgeDiff::compute()` gives us the union of all affected nodes.

### Why Siblings Need Special Handling

`EdgeDiff` captures changes to `Calls`/`Extends`/`Implements` edges. But sibling relationships are implicit — they're derived from shared `Contains` parent, not explicit edges between siblings. When node X is added/removed from a container, other nodes in that container have stale `siblings:` lists.

`get_embedding_siblings()` queries the `Contains` edges to find nodes sharing a parent container with any changed node. This is a 2-hop traversal: changed node → parent (via incoming `Contains`) → other children (via outgoing `Contains`).

## New Components

### 1. `get_embedding_neighbors()` in `codegraph-graph`

New function in `GraphTraverser` that computes the 1-hop embedding neighbor set:

```
Input: set of node IDs
Output: set of node IDs whose embedding text could reference any input node

Traversals:
  - Incoming Calls  → callers that list input as callee
  - Outgoing Calls  → callees that list input as caller
  - Incoming Extends → subclasses that list input as base
  - Incoming Implements → implementers that list input as interface
```

This mirrors the edge types used in `build_graph_context()` in `codegraph-core/src/embedding.rs`.

### 2. `get_embedding_siblings()` in `codegraph-graph`

New function for sibling detection:

```
Input: set of node IDs
Output: set of node IDs sharing a Contains parent with any input node

Traversal:
  - Input node → parent (incoming Contains edge where input is target)
  - Parent → children (outgoing Contains edges)
  - Return children minus input set
```

### 3. `sync_embeddings()` in `codegraph-core`

New method on `CodeGraph` that orchestrates the incremental embedding update. Called from `CodeGraph::sync()` after reference resolution. Returns `EmbeddingSyncResult`:

```rust
pub struct EmbeddingSyncResult {
    pub vectors_deleted: usize,
    pub vectors_created: usize,
    pub vectors_updated: usize,   // ripple re-embeds
    pub skipped_no_model: bool,
}
```

### 4. Extended `SyncResult`

`SyncResult` needs to carry the information needed for embedding updates:

```rust
pub struct SyncResult {
    pub stats: SyncStats,
    pub had_changes: bool,
    pub duration_ms: u64,
    pub enrichment_scope: SelectiveScope,
    pub deleted_node_ids: Vec<String>,    // NEW: nodes removed during sync
    pub changed_file_paths: Vec<String>,  // NEW: all files that changed (for EdgeSnapshot)
}
```

`deleted_node_ids` is collected from `process_modify()` and `process_delete()` which already fetch old nodes before deletion.

## Locking

The current index lock is acquired inside `SyncManager::sync_with_codegraph_dir()` and dropped when it returns. Embedding must happen under the same lock to prevent a concurrent writer from mutating the graph between sync and embedding.

**Option chosen:** Pass the lock scope up to `CodeGraph::sync()`. The lock is acquired before sync starts and held through embedding completion. `SyncManager` accepts an optional pre-acquired lock instead of acquiring its own.

## Graceful Degradation

- If ONNX feature is not enabled → skip embedding (log debug message)
- If model file not found → skip embedding (log info message suggesting model placement)
- If no changes detected → skip embedding entirely
- If embedding fails for a single node → log warning, continue with remaining nodes

This matches the current `generate_embeddings()` behavior in `codegraph-core`.

## Performance Considerations

- `EdgeSnapshot::capture_for_files()` uses a single SQL query with JOIN — efficient for bounded file sets
- `EdgeDiff::compute()` is set operations on in-memory `HashSet` — fast
- Embedding generation dominates cost; CoreML acceleration on Apple Silicon keeps per-node time low
- For large change sets (>100 files), consider falling back to full re-embed rather than computing precise diffs
- Filter to `EMBEDDABLE_KINDS` at the last step to avoid missing embeddable neighbors of non-embeddable changes

## Implementation Order

1. **Extend `SyncResult`** with `deleted_node_ids` and `changed_file_paths`
2. **Capture old node IDs** in `process_modify()` and `process_delete()` before deletion
3. **Add `get_embedding_neighbors()`** to `GraphTraverser` in `codegraph-graph`
4. **Add `get_embedding_siblings()`** to `GraphTraverser` in `codegraph-graph`
5. **Add `VectorStorage::delete_batch()`** for efficient bulk deletion
6. **Add `sync_embeddings()`** to `CodeGraph` in `codegraph-core`
7. **Wire up in `CodeGraph::sync()`** — EdgeSnapshot before/after, EdgeDiff, embedding update
8. **Extend locking** — hold index lock across sync + embedding
9. **Integrate `reembed.rs`** — check `should_full_reembed()` before incremental path
10. **Update `SyncStats`** with embedding counts for CLI output
