# Rust Bug Audit (Workspace Deep Dive)

Date: 2026-02-10

## Scope and method

This pass focused on all Rust crates in the workspace and covered:

1. Bug checks (runtime correctness risks + API contract bugs).
2. Maintainability review (module size/complexity, warning signal).
3. Architecture and idiomatic Rust adherence.
4. Inter-crate interface quality and dependency structure.

Crates reviewed:
- codegraph-types
- codegraph-db
- codegraph-extraction
- codegraph-resolution
- codegraph-graph
- codegraph-vectors
- codegraph-context
- codegraph-sync
- codegraph-lsp
- codegraph-mcp
- codegraph-core
- codegraph-cli

## Commands used

```bash
cargo test --workspace
cargo clippy --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo metadata --format-version 1 --no-deps
```

## High-priority bugs

## 1) Rust ONNX embedding path does not perform mean pooling as intended

- **Severity:** High
- **Crate:** `codegraph-vectors`
- **Where:** `embed_onnx` in `crates/codegraph-vectors/src/embedder.rs`
- **Evidence:** Code truncates with `take(dimension)` before checking `if embedding.len() > dimension`; this condition is unreachable and pooling branch is effectively dead.
- **Impact:** Models returning token-level tensors can degrade retrieval quality by using an unintended slice instead of pooled sentence embeddings.
- **Fix direction:** Drive extraction by output tensor shape; apply explicit pooling before final dimension checks.

## 2) Query/document prompt mode mismatch in embedding/search path

- **Severity:** Medium-High
- **Crates:** `codegraph-vectors` (+ parity impact with TypeScript)
- **Where:** `search_by_text` calls generic `embed(query)` in `crates/codegraph-vectors/src/search.rs`
- **Evidence:** Rust exposes one raw-text embedding path for search queries; TypeScript uses explicit query/document prefixes.
- **Impact:** Lower semantic alignment and search relevance on models tuned for dual prompt modes.
- **Fix direction:** Add `embed_query` and `embed_document` APIs and enforce consistent prompt formatting.

## 3) Ambiguous API contract in embedding storage

- **Severity:** Medium
- **Crate:** `codegraph-core`
- **Where:** `store_embedding(&self, node_id, embedding, text)` in `crates/codegraph-core/src/codegraph.rs`
- **Evidence:** `text` is forwarded into vector storage `model` field.
- **Impact:** Model provenance can be polluted by arbitrary caller input; weakens re-embed/version workflows.
- **Fix direction:** Rename to `model_id`, validate allowed identifiers, and expose strongly-typed metadata.

## 4) MCP contract drift breaks TS-era clients

- **Severity:** High
- **Crate:** `codegraph-mcp`
- **Where:** Tool schemas in `crates/codegraph-mcp/src/tools.rs`
- **Evidence:** Rust requires `node_id` where TypeScript-era contracts and instructions commonly use `symbol`; context key is `query` vs `task`.
- **Impact:** Runtime parameter errors for existing client flows.
- **Fix direction:** Support both key forms during migration and normalize internally.

## 5) `CodeGraph::init()` canonicalization friction on not-yet-existing paths

- **Severity:** Medium
- **Crate:** `codegraph-core`
- **Where:** `crates/codegraph-core/src/codegraph.rs`
- **Evidence:** Canonicalization occurs before creation, making initialization stricter than TS flow when target path does not exist.
- **Impact:** Migration friction and surprising init failures.
- **Fix direction:** Create missing path (or canonicalize parent then append).

## 6) CI strict-lint blocker in shared type constructor

- **Severity:** Medium
- **Crate:** `codegraph-types`
- **Where:** `Node::new(...)` in `crates/codegraph-types/src/lib.rs`
- **Evidence:** `clippy::too_many_arguments` fails when linting with `-D warnings`.
- **Impact:** Blocks strict CI and encourages continued constructor growth.
- **Fix direction:** Builder pattern or grouped constructor structs.

---

## Crate-by-crate deep-dive findings

## codegraph-types

- **Bug/quality findings:** `Node::new` arity exceeds idiomatic limits; strict clippy fails.
- **Maintainability:** Single large `lib.rs` (high LOC) concentrates many concerns (types + config + tests), increasing review and change friction.
- **Architecture/idiomatic:** Strong type centralization is good, but the crate would benefit from internal module splits (`node.rs`, `edge.rs`, `config.rs`).

## codegraph-db

- **Bug/quality findings:** Multiple clippy `redundant_closure` warnings and a very large query layer increase accidental complexity.
- **Maintainability:** `queries.rs` is a high-density “god module”; query, mapping, and API concerns are interleaved.
- **Architecture/idiomatic:** Solid use of `rusqlite` + typed row mapping, but should split query domains (nodes/edges/files/unresolved refs) into dedicated modules.

## codegraph-extraction

- **Bug/quality findings:** Multiple `too_many_arguments` and recursion-shape warnings indicate brittle traversal signatures; existing call/inheritance extraction remains complexity-heavy.
- **Maintainability:** Largest crate by LOC with heterogeneous responsibilities (scanner/parser/language config/tree walking/orchestration).
- **Architecture/idiomatic:** Good feature-gated grammar strategy; however, recursive APIs should be refactored around context structs to reduce argument sprawl.

## codegraph-resolution

- **Bug/quality findings:** Functional framework resolution breadth is limited versus TS registry model (higher false-negative risk for framework-heavy repos).
- **Maintainability:** Reasonably compact and readable.
- **Architecture/idiomatic:** Strategy chaining is clean; next step is trait-based framework plugins rather than growing inline heuristics.

## codegraph-graph

- **Bug/quality findings:** Mostly warning-level issues (unused imports/mutability in tests), but this signals incomplete API cleanup.
- **Maintainability:** Traversal/query logic is understandable; test coverage appears healthy.
- **Architecture/idiomatic:** Clear separation of traversal and higher-level queries; good crate boundary with DB.

## codegraph-vectors

- **Bug/quality findings:** Contains the highest-impact runtime bug in this pass (dead pooling branch), plus prompt-mode mismatch risk.
- **Maintainability:** Moderate complexity; ONNX session setup + output parsing would benefit from a stricter typed output adapter.
- **Architecture/idiomatic:** Good separation (embedder/storage/search/text_builder), but embedding API should encode query/document semantics explicitly.

## codegraph-context

- **Bug/quality findings:** Clippy reports nested-control and type-complexity smells; these are maintainability warnings rather than confirmed runtime bugs.
- **Maintainability:** Builder + formatter split is good, but callback signature complexity should be aliased with type definitions.
- **Architecture/idiomatic:** Reasonably idiomatic; improve readability via smaller helper units and reduced nested control flow.

## codegraph-sync

- **Bug/quality findings:** `repo_path` dead field and `if let Ok(..)` row iteration patterns indicate stale/fragile code paths.
- **Maintainability:** Good module breadth but some modules need cleanup to reduce silent-error handling patterns.
- **Architecture/idiomatic:** Sync pipeline is useful; use iterator combinators (`flatten`) and explicit error accounting to avoid accidental data drops.

## codegraph-lsp

- **Bug/quality findings:** No high-confidence functional bug identified in this pass.
- **Maintainability:** Broad module set (client/lifecycle/language adapters) is structurally good.
- **Architecture/idiomatic:** Async design is appropriate; ensure lifecycle and timeout policies are consistently documented/tested across adapters.

## codegraph-mcp

- **Bug/quality findings:** Most material client-facing contract drift lives here (parameter schema mismatch).
- **Maintainability:** Tool definitions and handlers are cohesive, but compatibility behavior should be centralized.
- **Architecture/idiomatic:** Good crate composition; add migration adapters for API stability.

## codegraph-core

- **Bug/quality findings:** Contains interface bugs (`init` canonicalization behavior, ambiguous embedding metadata parameter).
- **Maintainability:** Core is a broad façade crate and risks becoming a chokepoint.
- **Architecture/idiomatic:** Dependency fan-in is expected, but should keep orchestration thin and push feature-specific details into leaf crates.

## codegraph-cli

- **Bug/quality findings:** No critical runtime bug identified in this pass.
- **Maintainability:** Relatively small, easy to reason about.
- **Architecture/idiomatic:** Clear boundary as binary crate; keep business logic delegated to `codegraph-core`/`codegraph-mcp`.

---

## Interface and dependency analysis

## Inter-crate communication model

Primary communication contracts:
1. `codegraph-types` shared DTOs/types used nearly everywhere.
2. `codegraph-db::QueryBuilder` as the dominant query/mutation surface.
3. `codegraph-core` as orchestration façade across extraction, resolution, graph, vectors, sync, and context.

## Dependency structure (local crate graph)

- **Leaf/foundation:** `codegraph-types`
- **Data layer:** `codegraph-db` (depends on types)
- **Domain engines:** extraction, resolution, vectors, sync, graph, context, lsp (mostly depend on types + db)
- **Protocol/runtime:** mcp (depends on context/graph/db/types)
- **Orchestration:** core (depends on almost all engines)
- **Binary:** cli (depends on core + mcp)

## Strengths

- Mostly acyclic workspace layering.
- Strong shared type system.
- Logical separation between extraction, resolution, traversal, vectors, and protocol serving.

## Structural risks

1. **Core fan-in bottleneck:** `codegraph-core` depends on nearly all crates, increasing churn blast radius.
2. **DB surface concentration:** many crates couple directly to `QueryBuilder`; schema/API changes propagate widely.
3. **Contract drift risk across boundaries:** MCP request schema and embedding API semantics are not uniformly versioned/migrated.
4. **Large-module maintainability risk:** `codegraph-types`, `codegraph-db`, `codegraph-extraction` carry concentrated complexity.

## Recommended architectural hardening

1. Version MCP contract and add compatibility parsing (`symbol`/`node_id`, `task`/`query`).
2. Introduce narrower DB access traits per domain to reduce broad `QueryBuilder` coupling.
3. Keep `codegraph-core` as orchestration-only, moving domain details back into specialized crates.
4. Split very large modules into domain files with explicit ownership boundaries.
5. Add workspace lint gates per crate (`-D warnings`) with a staged cleanup plan.

---

## Maintainability and idiomatic Rust scorecard (qualitative)

- `codegraph-types`: **Medium** (good type base, needs modularization + constructor redesign)
- `codegraph-db`: **Medium** (solid SQL layer, needs decomposition and warning cleanup)
- `codegraph-extraction`: **Medium-Low** (feature-rich but high complexity)
- `codegraph-resolution`: **Medium-High** (clean strategy flow; framework breadth gap)
- `codegraph-graph`: **Medium-High** (clear boundaries; cleanup debt mostly minor)
- `codegraph-vectors`: **Medium** (good modularity, but contains high-impact correctness issue)
- `codegraph-context`: **Medium** (good separation, some complexity smells)
- `codegraph-sync`: **Medium** (useful architecture; some stale/error-handling smells)
- `codegraph-lsp`: **Medium-High** (modular async architecture)
- `codegraph-mcp`: **Medium** (good structure, compatibility risk)
- `codegraph-core`: **Medium** (valuable façade, but fan-in and API sharp edges)
- `codegraph-cli`: **High** (small and focused)

## Test/check summary for this audit revision

- `cargo test --workspace` ✅
- `cargo clippy --workspace --all-targets` ✅ (with warnings)
- `cargo clippy --workspace --all-targets -- -D warnings` ❌ (fails on existing lint issues, notably constructor arity)

