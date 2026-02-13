# Incremental Embedding Sync — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make `codegraph sync` incrementally update embeddings for new, modified, and deleted files — plus ripple-affected neighbors.

**Architecture:** `CodeGraph::sync()` in `codegraph-core` orchestrates embedding updates using `EdgeDiff` (pre/post snapshots), new `get_embedding_neighbors()`/`get_embedding_siblings()` in `codegraph-graph`, and existing `VectorStorage` + `reembed.rs` infrastructure.

**Tech Stack:** Rust, SQLite, `codegraph-graph` traversal, `codegraph-vectors` storage, `codegraph-sync` edge diff

**Design doc:** `docs/plans/2026-02-12-incremental-embedding-sync-design.md`

---

### Task 1: Extend SyncResult with deleted_node_ids and changed_file_paths

**Files:**
- Modify: `crates/codegraph-sync/src/sync.rs` (SyncResult struct + sync methods)

**Step 1: Add fields to SyncResult**

Add two new fields to `SyncResult`:

```rust
pub struct SyncResult {
    pub stats: SyncStats,
    pub had_changes: bool,
    pub duration_ms: u64,
    pub enrichment_scope: SelectiveScope,
    pub deleted_node_ids: Vec<String>,      // NEW
    pub changed_file_paths: Vec<String>,    // NEW
}
```

Update all places that construct `SyncResult` to include the new fields (initialized as empty vecs for now).

**Step 2: Run tests to verify nothing breaks**

Run: `cargo test --message-format=json -p codegraph-sync`
Expected: All existing tests PASS (new fields are just empty vecs)

**Step 3: Commit**

```
feat(sync): add deleted_node_ids and changed_file_paths to SyncResult
```

---

### Task 2: Capture old node IDs before deletion in process_modify and process_delete

**Files:**
- Modify: `crates/codegraph-sync/src/sync.rs:242-401` (process_changes, process_modify, process_delete)

**Step 1: Write failing test**

Add to `crates/codegraph-sync/src/sync.rs` tests:

```rust
#[test]
fn test_sync_captures_deleted_node_ids() {
    let dir = setup_test_dir();
    let db = DatabaseConnection::open_in_memory().unwrap();
    let mut queries = QueryBuilder::new(db.conn()).unwrap();

    let manager = SyncManager::new(dir.path().to_str().unwrap());

    // First sync - adds test.rs with its nodes
    let _ = manager.sync(db.conn(), &mut queries).unwrap();

    // Delete the file
    let file = dir.path().join("test.rs");
    std::fs::remove_file(&file).unwrap();

    // Second sync should capture deleted node IDs
    let result = manager.sync(db.conn(), &mut queries).unwrap();
    assert!(result.had_changes);
    assert!(!result.deleted_node_ids.is_empty(), "Should capture deleted node IDs");
}

#[test]
fn test_sync_modified_captures_old_node_ids() {
    let dir = setup_test_dir();
    let db = DatabaseConnection::open_in_memory().unwrap();
    let mut queries = QueryBuilder::new(db.conn()).unwrap();

    let manager = SyncManager::new(dir.path().to_str().unwrap());

    // First sync
    let _ = manager.sync(db.conn(), &mut queries).unwrap();

    // Modify file (completely different content)
    let file = dir.path().join("test.rs");
    fs::write(&file, "fn completely_new() {}").unwrap();

    // Second sync should capture old node IDs that were replaced
    let result = manager.sync(db.conn(), &mut queries).unwrap();
    assert!(result.had_changes);
    assert!(!result.deleted_node_ids.is_empty(), "Should capture old node IDs from modified file");
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test --message-format=json -p codegraph-sync test_sync_captures_deleted_node_ids`
Expected: FAIL (deleted_node_ids is empty)

**Step 3: Implement — collect deleted node IDs**

In `process_changes`, accumulate deleted node IDs from `process_modify` and `process_delete`. Both methods already call `queries.get_nodes_by_file()` before deletion — collect those IDs.

Modify `process_changes` signature to return `Vec<String>` of deleted IDs. In `process_modify`, the old nodes fetched at line 349 provide the IDs. In `process_delete`, the old nodes fetched at line 394 provide the IDs. Propagate these back to `sync_with_codegraph_dir` and store in `SyncResult.deleted_node_ids`.

Also populate `SyncResult.changed_file_paths` from the `changed_files` vec already computed at line 158.

**Step 4: Run tests to verify they pass**

Run: `cargo test --message-format=json -p codegraph-sync`
Expected: All PASS including new tests

**Step 5: Commit**

```
feat(sync): capture deleted node IDs and changed file paths during sync
```

---

### Task 3: Add get_embedding_neighbors() to GraphTraverser

**Files:**
- Modify: `crates/codegraph-graph/src/traversal.rs:286-495` (after embedding context methods)

**Step 1: Write failing test**

Add to `crates/codegraph-graph/src/traversal.rs` tests:

```rust
#[test]
fn test_get_embedding_neighbors_callers() {
    // A calls B: if B changes, A needs re-embedding (A lists B as callee)
    let (db, mut queries) = setup_test_graph();
    let mut traverser = GraphTraverser::new(db.conn(), &mut queries);

    let neighbors = traverser.get_embedding_neighbors(&["b"]).unwrap();

    // A calls B, so A is a neighbor (its embedding lists B as callee)
    assert!(neighbors.contains("a"), "Caller A should be a neighbor of B");
}

#[test]
fn test_get_embedding_neighbors_callees() {
    // A calls B: if A changes, B needs re-embedding (B lists A as caller)
    let (db, mut queries) = setup_test_graph();
    let mut traverser = GraphTraverser::new(db.conn(), &mut queries);

    let neighbors = traverser.get_embedding_neighbors(&["a"]).unwrap();

    // A calls B and D, so B and D are neighbors (their embeddings list A as caller)
    assert!(neighbors.contains("b"), "Callee B should be a neighbor of A");
    assert!(neighbors.contains("d"), "Callee D should be a neighbor of A");
}

#[test]
fn test_get_embedding_neighbors_extends() {
    let (db, mut queries) = setup_inheritance_graph();
    let mut traverser = GraphTraverser::new(db.conn(), &mut queries);

    // If BaseService changes, PaymentService needs re-embedding (lists BaseService as extends)
    let neighbors = traverser.get_embedding_neighbors(&["base_service"]).unwrap();
    assert!(neighbors.contains("payment_service"));
}

#[test]
fn test_get_embedding_neighbors_implements() {
    let (db, mut queries) = setup_inheritance_graph();
    let mut traverser = GraphTraverser::new(db.conn(), &mut queries);

    // If IPayment changes, PaymentService needs re-embedding (lists IPayment as implements)
    let neighbors = traverser.get_embedding_neighbors(&["i_payment"]).unwrap();
    assert!(neighbors.contains("payment_service"));
}

#[test]
fn test_get_embedding_neighbors_excludes_input() {
    let (db, mut queries) = setup_test_graph();
    let mut traverser = GraphTraverser::new(db.conn(), &mut queries);

    let neighbors = traverser.get_embedding_neighbors(&["b"]).unwrap();
    // Should not contain the input node itself
    assert!(!neighbors.contains("b"));
}

#[test]
fn test_get_embedding_neighbors_multiple_inputs() {
    let (db, mut queries) = setup_test_graph();
    let mut traverser = GraphTraverser::new(db.conn(), &mut queries);

    let neighbors = traverser.get_embedding_neighbors(&["b", "d"]).unwrap();
    // A calls both B and D, so A should appear
    assert!(neighbors.contains("a"));
    // C is called by B, so C should appear
    assert!(neighbors.contains("c"));
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test --message-format=json -p codegraph-graph test_get_embedding_neighbors`
Expected: FAIL (method doesn't exist)

**Step 3: Implement get_embedding_neighbors**

Add to `GraphTraverser` after the existing embedding context methods:

```rust
/// Get all nodes whose embedding text could reference any of the given node IDs.
///
/// This is a 1-hop traversal aligned with build_graph_context():
/// - Incoming Calls → callers that list input as callee
/// - Outgoing Calls → callees that list input as caller
/// - Incoming Extends → subclasses that list input as base
/// - Incoming Implements → implementers that list input as interface
///
/// Returns node IDs excluding the input set.
pub fn get_embedding_neighbors(
    &mut self,
    node_ids: &[&str],
) -> Result<HashSet<String>, GraphError> {
    let input_set: HashSet<&str> = node_ids.iter().copied().collect();
    let mut neighbors: HashSet<String> = HashSet::new();

    for &node_id in node_ids {
        // Incoming Calls: callers whose embedding says "calls: <node_id>"
        let incoming_calls = self.queries.get_incoming_edges(
            self.conn, node_id, Some(&[EdgeKind::Calls]),
        )?;
        for edge in &incoming_calls {
            neighbors.insert(edge.source.0.clone());
        }

        // Outgoing Calls: callees whose embedding says "called by: <node_id>"
        let outgoing_calls = self.queries.get_outgoing_edges(
            self.conn, node_id, Some(&[EdgeKind::Calls]),
        )?;
        for edge in &outgoing_calls {
            neighbors.insert(edge.target.0.clone());
        }

        // Incoming Extends: subclasses whose embedding says "extends: <node_id>"
        let incoming_extends = self.queries.get_incoming_edges(
            self.conn, node_id, Some(&[EdgeKind::Extends]),
        )?;
        for edge in &incoming_extends {
            neighbors.insert(edge.source.0.clone());
        }

        // Incoming Implements: implementers whose embedding says "implements: <node_id>"
        let incoming_implements = self.queries.get_incoming_edges(
            self.conn, node_id, Some(&[EdgeKind::Implements]),
        )?;
        for edge in &incoming_implements {
            neighbors.insert(edge.source.0.clone());
        }
    }

    // Remove input nodes from result
    for &id in &input_set {
        neighbors.remove(id);
    }

    Ok(neighbors)
}
```

**Step 4: Run tests to verify they pass**

Run: `cargo test --message-format=json -p codegraph-graph`
Expected: All PASS

**Step 5: Commit**

```
feat(graph): add get_embedding_neighbors for 1-hop ripple detection
```

---

### Task 4: Add get_embedding_siblings() to GraphTraverser

**Files:**
- Modify: `crates/codegraph-graph/src/traversal.rs` (after get_embedding_neighbors)

**Step 1: Write failing test**

```rust
#[test]
fn test_get_embedding_siblings() {
    let (db, mut queries) = setup_container_graph();
    let mut traverser = GraphTraverser::new(db.conn(), &mut queries);

    // If method1 changes, method2 and method3 need re-embedding (stale siblings list)
    let siblings = traverser.get_embedding_siblings(&["method1"]).unwrap();
    assert!(siblings.contains("method2"));
    assert!(siblings.contains("method3"));
    assert!(!siblings.contains("method1"), "Should exclude input");
    assert!(!siblings.contains("my_class"), "Should exclude parent container");
}

#[test]
fn test_get_embedding_siblings_no_container() {
    let (db, mut queries) = setup_test_graph();
    let mut traverser = GraphTraverser::new(db.conn(), &mut queries);

    // Standalone functions with no container have no siblings
    let siblings = traverser.get_embedding_siblings(&["a"]).unwrap();
    assert!(siblings.is_empty());
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test --message-format=json -p codegraph-graph test_get_embedding_siblings`
Expected: FAIL (method doesn't exist)

**Step 3: Implement get_embedding_siblings**

```rust
/// Get all nodes that share a Contains parent with any of the given node IDs.
///
/// When a node is added/removed from a container, other nodes in that container
/// have stale "siblings:" lists in their embeddings.
///
/// Traversal: input → parent (incoming Contains) → children (outgoing Contains) - input
pub fn get_embedding_siblings(
    &mut self,
    node_ids: &[&str],
) -> Result<HashSet<String>, GraphError> {
    let input_set: HashSet<&str> = node_ids.iter().copied().collect();
    let mut siblings: HashSet<String> = HashSet::new();

    for &node_id in node_ids {
        // Find parent containers
        let parent_edges = self.queries.get_incoming_edges(
            self.conn, node_id, Some(&[EdgeKind::Contains]),
        )?;

        for parent_edge in parent_edges {
            // Get all children of this parent
            let child_edges = self.queries.get_outgoing_edges(
                self.conn, &parent_edge.source.0, Some(&[EdgeKind::Contains]),
            )?;

            for child_edge in child_edges {
                let child_id = &child_edge.target.0;
                if !input_set.contains(child_id.as_str()) {
                    siblings.insert(child_id.clone());
                }
            }
        }
    }

    // Also exclude parent containers themselves
    Ok(siblings)
}
```

**Step 4: Run tests to verify they pass**

Run: `cargo test --message-format=json -p codegraph-graph`
Expected: All PASS

**Step 5: Commit**

```
feat(graph): add get_embedding_siblings for container-based ripple detection
```

---

### Task 5: Add VectorStorage::delete_batch()

**Files:**
- Modify: `crates/codegraph-vectors/src/storage.rs`

**Step 1: Write failing test**

Add to existing tests in `storage.rs`:

```rust
#[test]
fn test_delete_batch_vectors() {
    let db = DatabaseConnection::open_in_memory().unwrap();
    let storage = VectorStorage::new(384);
    storage.init(db.conn()).unwrap();

    let vector: Vec<f32> = vec![0.0; 384];
    storage.store(db.conn(), "n1", &vector, "test").unwrap();
    storage.store(db.conn(), "n2", &vector, "test").unwrap();
    storage.store(db.conn(), "n3", &vector, "test").unwrap();

    assert_eq!(storage.count(db.conn()).unwrap(), 3);

    storage.delete_batch(db.conn(), &["n1", "n3"]).unwrap();

    assert_eq!(storage.count(db.conn()).unwrap(), 1);
    assert!(storage.get(db.conn(), "n1").unwrap().is_none());
    assert!(storage.get(db.conn(), "n2").unwrap().is_some());
    assert!(storage.get(db.conn(), "n3").unwrap().is_none());
}

#[test]
fn test_delete_batch_empty() {
    let db = DatabaseConnection::open_in_memory().unwrap();
    let storage = VectorStorage::new(384);
    storage.init(db.conn()).unwrap();

    // Should not error on empty input
    storage.delete_batch(db.conn(), &[]).unwrap();
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test --message-format=json -p codegraph-vectors test_delete_batch`
Expected: FAIL (method doesn't exist)

**Step 3: Implement delete_batch**

Add to `VectorStorage`:

```rust
/// Delete vectors for multiple nodes
pub fn delete_batch(&self, conn: &Connection, node_ids: &[&str]) -> Result<(), VectorError> {
    if node_ids.is_empty() {
        return Ok(());
    }

    let placeholders: String = node_ids
        .iter()
        .enumerate()
        .map(|(i, _)| format!("?{}", i + 1))
        .collect::<Vec<_>>()
        .join(",");

    let sql = format!("DELETE FROM vectors WHERE node_id IN ({})", placeholders);
    let mut stmt = conn.prepare(&sql)?;
    let params: Vec<&dyn rusqlite::ToSql> = node_ids
        .iter()
        .map(|id| id as &dyn rusqlite::ToSql)
        .collect();
    stmt.execute(params.as_slice())?;

    Ok(())
}
```

**Step 4: Run tests to verify they pass**

Run: `cargo test --message-format=json -p codegraph-vectors`
Expected: All PASS

**Step 5: Commit**

```
feat(vectors): add delete_batch to VectorStorage for bulk cleanup
```

---

### Task 6: Add sync_embeddings() to CodeGraph

**Files:**
- Modify: `crates/codegraph-core/src/codegraph.rs` (new method + result struct)

**Step 1: Add EmbeddingSyncResult struct and sync_embeddings method**

Add after the `generate_embeddings` method:

```rust
/// Result of incremental embedding sync
#[derive(Debug, Default)]
pub struct EmbeddingSyncResult {
    /// Vectors deleted (for removed nodes)
    pub vectors_deleted: usize,
    /// Vectors created or updated
    pub vectors_upserted: usize,
    /// Skipped because model not available
    pub skipped_no_model: bool,
}
```

Implement `sync_embeddings`:

```rust
/// Incrementally update embeddings after a sync operation
///
/// 1. Deletes vectors for removed nodes
/// 2. Generates embeddings for new/modified nodes
/// 3. Re-embeds ripple-affected neighbors (callers, callees, siblings)
fn sync_embeddings(
    &mut self,
    deleted_node_ids: &[String],
    embed_candidate_ids: &HashSet<String>,
) -> Result<EmbeddingSyncResult, CodeGraphError> {
    let mut result = EmbeddingSyncResult::default();

    // Load embedder (graceful skip if unavailable)
    let mut embedder = TextEmbedder::new(EmbedderConfig::default());
    match embedder.load() {
        Ok(()) => {}
        Err(VectorError::ModelNotFound { .. }) => {
            log::info!("Embedding model not found, skipping incremental embedding");
            result.skipped_no_model = true;
            return Ok(result);
        }
        Err(VectorError::FeatureNotEnabled { .. }) => {
            log::debug!("ONNX feature not enabled, skipping incremental embedding");
            result.skipped_no_model = true;
            return Ok(result);
        }
        Err(e) => return Err(CodeGraphError::Vector(e)),
    }

    let dimension = embedder.dimension();
    let storage = VectorStorage::new(dimension);
    storage.init(self.db.conn())?;
    let model_name = "nomic-embed-text-v1.5";

    // 1. Delete stale vectors
    if !deleted_node_ids.is_empty() {
        let refs: Vec<&str> = deleted_node_ids.iter().map(|s| s.as_str()).collect();
        storage.delete_batch(self.db.conn(), &refs)?;
        result.vectors_deleted = deleted_node_ids.len();
        log::info!("Deleted {} stale embeddings", result.vectors_deleted);
    }

    // 2. Generate embeddings for candidates
    for node_id in embed_candidate_ids {
        let node = match self.queries.get_node_by_id(self.db.conn(), node_id)? {
            Some(n) => n,
            None => continue, // Node was deleted, skip
        };

        // Filter to embeddable kinds
        if !Self::EMBEDDABLE_KINDS.contains(&node.kind) {
            continue;
        }

        let text = match crate::embedding::build_embedding_text(
            self.db.conn(),
            &mut self.queries,
            &node,
            &self.config.embedding,
        ) {
            Ok(text) => text,
            Err(e) => {
                log::debug!("Failed to build embedding text for {}: {}", node_id, e);
                continue;
            }
        };

        let embedding = match embedder.embed_document(&text) {
            Ok(e) => e,
            Err(e) => {
                log::warn!("Failed to embed {}: {}", node_id, e);
                continue;
            }
        };

        storage.store(self.db.conn(), node_id, &embedding, model_name)?;
        result.vectors_upserted += 1;
    }

    log::info!(
        "Incremental embedding sync: {} deleted, {} upserted",
        result.vectors_deleted,
        result.vectors_upserted,
    );

    Ok(result)
}
```

**Step 2: Run cargo check to verify it compiles**

Run: `cargo check --message-format=json -p codegraph-core`
Expected: PASS (no callers yet, just compilation check)

**Step 3: Commit**

```
feat(core): add sync_embeddings method for incremental embedding updates
```

---

### Task 7: Wire up incremental embedding sync in CodeGraph::sync()

**Files:**
- Modify: `crates/codegraph-core/src/codegraph.rs:248-275` (the sync method)

**Step 1: Implement the full wiring**

Replace the current `CodeGraph::sync()` with the full data flow:

```rust
pub fn sync(&mut self) -> Result<codegraph_sync::SyncResult, CodeGraphError> {
    use codegraph_sync::{EdgeSnapshot, EdgeDiff, SyncConfig, SyncManager};

    let sync_config = SyncConfig {
        excludes: self.config.exclude_patterns.clone(),
        continue_on_error: true,
        ..SyncConfig::default()
    };

    let manager = SyncManager::with_config(
        self.config.root.to_string_lossy().to_string(),
        sync_config,
    );

    // Step 1: Detect changes (without processing) to get file list
    // We need changed_file_paths BEFORE sync to take pre-snapshot
    // For now, run sync and use its changed_file_paths for post-snapshot
    // Pre-snapshot uses the same file list (captured inside sync before deletion)

    let result = manager.sync_with_codegraph_dir(
        self.db.conn(),
        &mut self.queries,
        Some(&self.config.data_dir),
    )?;

    // Resolve references for changed files if enabled
    if self.config.resolve_references && result.had_changes {
        let mut resolver = ReferenceResolver::new(self.db.conn(), &mut self.queries);
        let _ = resolver.resolve_all();
    }

    // Incremental embedding update
    if result.had_changes {
        // Compute ripple set from edge diff
        let changed_file_refs: Vec<&str> = result.changed_file_paths.iter()
            .map(|s| s.as_str()).collect();

        // Post-sync snapshot (pre-snapshot is captured inside SyncManager - see Task 8)
        let post_snapshot = EdgeSnapshot::capture_for_files(
            self.db.conn(), &changed_file_refs,
        );

        // Get new node IDs from changed files
        let mut embed_candidates: std::collections::HashSet<String> =
            std::collections::HashSet::new();

        // Add all current nodes in changed files
        for file_path in &result.changed_file_paths {
            let nodes = self.queries.get_nodes_by_file(self.db.conn(), file_path)?;
            for node in nodes {
                embed_candidates.insert(node.id.0.clone());
            }
        }

        // Compute neighbors and siblings of changed/deleted nodes
        {
            let all_changed: Vec<&str> = embed_candidates.iter()
                .map(|s| s.as_str())
                .chain(result.deleted_node_ids.iter().map(|s| s.as_str()))
                .collect();

            let mut traverser = GraphTraverser::new(self.db.conn(), &mut self.queries);

            let neighbors = traverser.get_embedding_neighbors(&all_changed)?;
            embed_candidates.extend(neighbors);

            let siblings = traverser.get_embedding_siblings(&all_changed)?;
            embed_candidates.extend(siblings);
        }

        // Remove deleted nodes from candidates (they'll be deleted, not embedded)
        for id in &result.deleted_node_ids {
            embed_candidates.remove(id);
        }

        // Run incremental embedding
        let _embed_result = self.sync_embeddings(
            &result.deleted_node_ids,
            &embed_candidates,
        )?;
    }

    Ok(result)
}
```

**Step 2: Run compilation check**

Run: `cargo check --message-format=json -p codegraph-core`
Expected: PASS

**Step 3: Run full test suite**

Run: `cargo test --message-format=json`
Expected: All PASS

**Step 4: Commit**

```
feat(core): wire incremental embedding sync into CodeGraph::sync()
```

---

### Task 8: Add pre-deletion EdgeSnapshot capture to SyncManager

**Files:**
- Modify: `crates/codegraph-sync/src/sync.rs` (add pre_edge_snapshot to SyncResult)

**Step 1: Add pre_edge_snapshot field to SyncResult**

```rust
pub struct SyncResult {
    pub stats: SyncStats,
    pub had_changes: bool,
    pub duration_ms: u64,
    pub enrichment_scope: SelectiveScope,
    pub deleted_node_ids: Vec<String>,
    pub changed_file_paths: Vec<String>,
    pub pre_edge_snapshot: Option<EdgeSnapshot>,  // NEW
}
```

**Step 2: Capture pre-snapshot before processing changes**

In `sync_with_codegraph_dir`, after detecting changes but before processing them, capture the edge snapshot:

```rust
// Capture pre-deletion edge snapshot for embedding ripple detection
let pre_edge_snapshot = if had_changes {
    let file_refs: Vec<&str> = changed_files.iter().map(|s| s.as_str()).collect();
    Some(EdgeSnapshot::capture_for_files(conn, &file_refs))
} else {
    None
};
```

Store this in the returned `SyncResult`.

**Step 3: Update CodeGraph::sync() to use pre-snapshot for EdgeDiff**

In `codegraph-core/src/codegraph.rs`, use the pre-snapshot from `SyncResult` to compute `EdgeDiff`:

```rust
if let Some(pre_snapshot) = &result.pre_edge_snapshot {
    let diff = EdgeDiff::compute(pre_snapshot, &post_snapshot);
    embed_candidates.extend(diff.affected_nodes);
}
```

**Step 4: Run tests**

Run: `cargo test --message-format=json`
Expected: All PASS

**Step 5: Commit**

```
feat(sync): capture pre-deletion edge snapshot for embedding ripple detection
```

---

### Task 9: Integrate reembed.rs for full vs incremental decision

**Files:**
- Modify: `crates/codegraph-core/src/codegraph.rs` (CodeGraph::sync method)

**Step 1: Add reembed trigger check at start of sync**

Before the incremental path, check if full re-embed is needed:

```rust
use codegraph_sync::reembed::{should_full_reembed, record_embed_metadata, ReembedConfig};

// Check if full re-embed is needed (schema/config/model changed)
let reembed_config = ReembedConfig {
    schema_version: "1",  // bump when schema changes
    config_json: &serde_json::to_string(&self.config.embedding).unwrap_or_default(),
    model_hash: "nomic-embed-text-v1.5",
    force: false,
};

if should_full_reembed(self.db.conn(), &self.queries, &reembed_config) {
    log::info!("Full re-embed triggered, running generate_embeddings");
    self.generate_embeddings()?;
    record_embed_metadata(self.db.conn(), &self.queries, &reembed_config)?;
    return Ok(result);
}
```

After successful incremental embed, record metadata:

```rust
record_embed_metadata(self.db.conn(), &self.queries, &reembed_config)?;
```

**Step 2: Run tests**

Run: `cargo test --message-format=json -p codegraph-core`
Expected: All PASS

**Step 3: Commit**

```
feat(core): integrate reembed triggers for full vs incremental decision
```

---

### Task 10: Update SyncStats with embedding counts for CLI output

**Files:**
- Modify: `crates/codegraph-sync/src/sync.rs` (SyncStats)
- Modify: `crates/codegraph-cli/src/commands.rs` (sync command output)

**Step 1: Add embedding fields to SyncStats**

```rust
pub struct SyncStats {
    // ... existing fields ...
    /// Embeddings deleted
    pub embeddings_deleted: usize,
    /// Embeddings created/updated
    pub embeddings_upserted: usize,
}
```

**Step 2: Update CLI output**

Find where sync stats are printed in the CLI and add embedding info:

```rust
if stats.embeddings_upserted > 0 || stats.embeddings_deleted > 0 {
    println!("  Embeddings: {} upserted, {} deleted",
        stats.embeddings_upserted, stats.embeddings_deleted);
}
```

**Step 3: Wire stats from EmbeddingSyncResult to SyncStats**

In `CodeGraph::sync()`, after `sync_embeddings` returns, update the stats on the result.

**Step 4: Run tests**

Run: `cargo test --message-format=json`
Expected: All PASS

**Step 5: Commit**

```
feat(cli): show embedding sync stats in sync command output
```

---

### Task 11: Integration test — end-to-end incremental embedding sync

**Files:**
- Create: `crates/codegraph-core/tests/embedding_sync_test.rs`

**Step 1: Write integration test**

```rust
//! Integration test for incremental embedding sync

use codegraph_core::CodeGraph;
use std::fs;
use tempfile::TempDir;

fn setup_project() -> (TempDir, CodeGraph) {
    let dir = tempfile::tempdir().unwrap();

    // Create initial files
    fs::write(
        dir.path().join("main.rs"),
        "fn main() { helper(); }\nfn helper() { println!(\"hello\"); }",
    ).unwrap();

    let cg = CodeGraph::init(dir.path()).unwrap();
    (dir, cg)
}

#[test]
fn test_sync_embeds_new_files() {
    let (dir, mut cg) = setup_project();

    // Full index first
    let _ = cg.index().unwrap();

    // Add a new file
    fs::write(dir.path().join("utils.rs"), "fn utility() {}").unwrap();

    // Sync should embed the new file
    let result = cg.sync().unwrap();
    assert!(result.had_changes);
    // The new nodes should have embeddings (verified by semantic search working)
}

#[test]
fn test_sync_removes_embeddings_for_deleted_files() {
    let (dir, mut cg) = setup_project();

    let _ = cg.index().unwrap();

    // Delete a file
    fs::remove_file(dir.path().join("main.rs")).unwrap();

    let result = cg.sync().unwrap();
    assert!(result.had_changes);
    assert!(!result.deleted_node_ids.is_empty());
}

#[test]
fn test_sync_re_embeds_modified_files() {
    let (dir, mut cg) = setup_project();

    let _ = cg.index().unwrap();

    // Modify file
    fs::write(
        dir.path().join("main.rs"),
        "fn main() { new_function(); }\nfn new_function() {}",
    ).unwrap();

    let result = cg.sync().unwrap();
    assert!(result.had_changes);
}
```

**Step 2: Run integration tests**

Run: `cargo test --message-format=json -p codegraph-core --test embedding_sync_test`
Expected: All PASS

**Step 3: Commit**

```
test(core): add integration tests for incremental embedding sync
```
