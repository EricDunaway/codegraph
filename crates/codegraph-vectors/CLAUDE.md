# codegraph-vectors

Vector embeddings and semantic search for CodeGraph. Handles ONNX model loading, embedding generation, SQLite-based vector storage, cosine similarity search, and enriched text construction with graph context and token budgeting.

## Crate Layout

```
src/
├── lib.rs           # Re-exports public API
├── embedder.rs      # TextEmbedder — ONNX model loading + inference
├── storage.rs       # VectorStorage — SQLite BLOB persistence
├── search.rs        # SimilaritySearch — brute-force cosine search
├── text_builder.rs  # EmbeddingTextBuilder — enriched text with token budget
├── error.rs         # VectorError variants
```

All paths below are relative to `crates/codegraph-vectors/`.

## TextEmbedder (`src/embedder.rs`)

Wraps ONNX inference via the `ort` crate. Two-phase lifecycle: `new()` then `load()`.

### Model resolution

`load()` resolves model files by searching (in order):
1. Explicit path from `EmbedderConfig.model_path` / `tokenizer_path`
2. `.codegraph/models/` (project-level)
3. `~/.codegraph/models/` (user-level)

Expected files: `nomic-embed-text-v1.5.onnx` and `tokenizer.json`.

### Embed methods

| Method | Prefix prepended | Use for |
|--------|-----------------|---------|
| `embed(text)` | none | Raw embedding, no prefix |
| `embed_query(query)` | `search_query: ` | Search queries (asymmetric retrieval) |
| `embed_document(doc)` | `search_document: ` | Stored documents (asymmetric retrieval) |
| `embed_batch(texts)` | none | Multiple texts, calls `embed()` per item |

### CoreML acceleration (Apple Silicon)

On `cfg(all(target_os = "macos", target_arch = "aarch64"))`, ort is compiled with the `coreml` feature. `build_session()` registers a CoreML execution provider for Neural Engine + GPU acceleration. Falls back to CPU if CoreML registration fails.

Set `CODEGRAPH_NO_COREML=1` to force CPU-only on Apple Silicon.

### Mock mode (no `onnx` feature)

Without the `onnx` feature flag, `embed()` returns a deterministic SHA256-based pseudo-random unit vector. `load()` returns `Err(VectorError::FeatureNotEnabled)`. `is_loaded()` always returns `false`.

## VectorStorage (`src/storage.rs`)

Stores embeddings in a `vectors` SQLite table as BLOBs (little-endian `f32` arrays).

### Schema

```sql
CREATE TABLE vectors (
    node_id TEXT PRIMARY KEY,
    vector BLOB NOT NULL,
    model TEXT NOT NULL,
    dimension INTEGER NOT NULL,
    created_at INTEGER NOT NULL  -- unix epoch seconds
);
CREATE INDEX idx_vectors_model ON vectors(model);
```

### Key methods

- `init(conn)` — creates table + index if not exists
- `store(conn, node_id, vector, model)` — upsert; errors on dimension mismatch
- `get(conn, node_id)` -> `Option<VectorRecord>`
- `get_all(conn)` -> `Vec<VectorRecord>` — loads all vectors into memory (used by search)
- `get_batch(conn, node_ids)` — parameterized IN query
- `delete(conn, node_id)` / `delete_batch(conn, node_ids)`
- `count(conn)` / `clear(conn)`

Vectors are serialized via `vector_to_blob()` / `blob_to_vector()` using `f32::to_le_bytes()`.

## SimilaritySearch (`src/search.rs`)

Brute-force cosine similarity search over all stored vectors.

### Usage

```rust
let mut search = SimilaritySearch::with_config(&storage, &mut embedder, SearchConfig {
    max_results: 10,
    min_score: 0.3,
});
let results = search.search_by_text(conn, "payment processing")?;
```

### Methods

- `search_by_text(conn, query)` — embeds query with `embed_query()` prefix, then calls `search_by_vector`
- `search_by_vector(conn, query_vector)` — loads all vectors via `get_all()`, computes cosine similarity, filters by `min_score`, sorts descending, truncates to `max_results`
- `find_similar(conn, node_id)` — finds neighbors of an existing node (excludes self)
- `search_in_subset(conn, query, node_ids)` — searches within a specific set of nodes

### Standalone functions

- `cosine_similarity(a, b)` -> `f32` — returns 0.0 on empty or mismatched lengths
- `euclidean_distance(a, b)` -> `f32`

## EmbeddingTextBuilder (`src/text_builder.rs`)

Constructs enriched text from a `Node` + `GraphContext` + `NodeEnrichment` for embedding.

### Text ordering

1. Decorators (E1: decorators first)
2. Kind + name (`function processPayment`)
3. File path (`in src/payment.ts`)
4. Signature
5. Docstring
6. Inferred type (from LSP)
7. Resolved import path (import nodes only)
8. Package name (enrichment or derived from parent directory)
9. Inheritance (`extends` / `implements`, class/struct nodes only)
10. Graph context (callees, callers, siblings — capped by config limits)
11. Thrown errors
12. Associated tests
13. Code snippet (last, most expendable)

### Token budgeting (`build_text_with_budget`)

Uses `tiktoken-rs` cl100k_base as a proxy tokenizer. If text exceeds `max_tokens`, truncation tiers apply:

| Tier | What gets removed |
|------|-------------------|
| 1 (B6) | Code snippet removed, decorators capped at 10, signature capped at 200 chars |
| 2 | All graph context removed (callees, callers, siblings) |
| 3 | Docstring + all enrichment removed |
| Fallback | Only `kind name\nin file_path` |

### GraphContext fields

- `callees`, `callers`, `siblings` — `Vec<String>`, capped by `EmbeddingTextConfig.max_callees/callers/siblings`
- `implements` — `Vec<String>`, `extends` — `Option<String>`

### NodeEnrichment fields

`inferred_type`, `resolved_import_path`, `package_name`, `thrown_errors`, `test_names`, `code_snippet`. Use `NodeEnrichment::from_node(node)` to populate from a `Node`'s enrichment fields.

## Model Details

- **Model**: nomic-embed-text-v1.5 (ONNX format)
- **Output dimension**: 768
- **Max sequence length**: 512 tokens
- **Prefix convention**: `search_query: ` for queries, `search_document: ` for stored documents
- **Output handling**: Mean-pools token-level output (`last_hidden_state` shape `[1, seq_len, 768]`); passes through sentence-level output directly
- **Integrity**: Optional SHA256 hash verification via `EmbedderConfig.model_hash`

## Feature Flags

| Flag | Dependencies added | Effect |
|------|-------------------|--------|
| `onnx` | `ort`, `tokenizers`, `ndarray` | Enables real ONNX inference. Without it, only mock embeddings available. |

CoreML is not a separate feature flag — it is auto-enabled at compile time on `target_os = "macos" + target_arch = "aarch64"` via target-specific `ort` dependency configuration in `Cargo.toml`.

## Gotchas

- **`load()` is expensive (~100ms)**: Creates an ONNX session. Do not call twice in the same code path. Check `is_loaded()` first.
- **`generate_embeddings()` returns `Ok(0)` ambiguity**: In `codegraph-core`, `Ok(0)` means either "model not found" or "no embeddable nodes". Check model availability separately if the distinction matters.
- **Brute-force search**: `search_by_vector` loads ALL vectors into memory via `get_all()`. Fine for typical project sizes, but scales linearly with vector count.
- **Mock embeddings are hash-based, not semantic**: Without the `onnx` feature, embeddings are deterministic but have no semantic meaning. Tests using mock mode should not assert on similarity quality.
- **`embed_batch` is sequential**: Calls `embed()` in a loop. No batched ONNX inference.
- **`SearchConfig.min_score` defaults to 0.0**: All results pass the threshold by default. Set this higher (e.g., 0.3) in practice.
