# TypeScript vs Rust Feature Discrepancies

Date: 2026-02-10

## Comparison basis

This compares current TypeScript implementation surface (`src/`) with current Rust crates (`crates/`) for user-visible behavior.

## Executive summary

Rust has strong crate coverage for core indexing/querying, but there are notable parity gaps in:

1. MCP tool parameter compatibility.
2. Framework-specific resolution breadth.
3. Extraction language support breadth.
4. Embedding flow behavior and model usage parity.
5. Installer/runtime integration from TS CLI UX.

---

## 1) MCP tool schema and calling convention drift

| Area | TypeScript | Rust | Discrepancy |
|---|---|---|---|
| `codegraph_callers`/`callees`/`impact`/`node` lookup key | `symbol` | `node_id` | Breaking change for callers expecting symbol-name API |
| `codegraph_context` key | `task` (+ `maxNodes`, `includeCode`) | `query` (+ `max_tokens`) | Different argument model and naming |
| Search filter | `kind` supported | no `kind` in tool schema | Reduced feature parity |
| Extra tool | no `codegraph_file_nodes` in TS MCP tool list | Rust adds `codegraph_file_nodes` | Rust-only capability (good addition, but not parity) |

**Implication:** Migration from TS MCP client assumptions to Rust MCP is non-drop-in.

---

## 2) Framework resolver breadth

TypeScript has a framework resolver registry spanning Laravel, Express, React, Django, Flask, FastAPI, Rails, Spring, Go, Rust, ASP.NET, SwiftUI/UIKit/Vapor.

Rust resolution currently includes generic strategies (import/exact/qualified/fuzzy) plus minimal framework heuristics (React-style component name and route-handler heuristic), not a registry with language/framework modules equivalent to TS.

**Gap:** TS framework-specialized resolution is broader and more explicit.

---

## 3) Extraction language support differences

TypeScript extraction grammar mapping includes: TypeScript/TSX, JavaScript/JSX, Python, Go, Rust, Java, C, C++, C#, PHP, Ruby, Swift, Kotlin, and custom Liquid handling.

Rust `get_language_config(...)` currently wires TypeScript/JS/TSX/JSX, Rust, Python, Go, PHP, Java, C#, Ruby, Dart, Swift, GraphQL, HCL (with Kotlin explicitly noted as blocked in comments).

**Observed discrepancies:**

- TS includes C/C++/Kotlin/Liquid extraction pathways absent from Rust language config routing.
- Rust includes Dart/GraphQL/HCL not present in TS grammar map.

**Implication:** Language support overlap is substantial but not symmetrical.

---

## 4) Embedding flow functionality + model usage differences

### 4.1 Embedding execution stack

| Area | TypeScript | Rust | Discrepancy |
|---|---|---|---|
| Runtime | `@xenova/transformers` pipeline | direct ONNX Runtime (`ort`) | Different inference stack and behavior surface |
| Model locator | model ID (`nomic-ai/nomic-embed-text-v1.5`) with cache | local ONNX file + tokenizer search | Different deployment/runtime assumptions |
| Integrity check | none in flow | optional SHA256 validation | Rust has stronger local artifact integrity controls |

### 4.2 Query vs document embedding behavior

| Area | TypeScript | Rust | Discrepancy |
|---|---|---|---|
| Query embedding | `embedQuery()` uses `search_query:` prefix | `search_by_text()` calls generic `embed(query)` | Rust lacks task-specific query prompt formatting |
| Document embedding | `embedBatch(..., 'document')` uses `search_document:` prefix | generic `embed(text)` path | Rust lacks explicit document/query mode split |

**Implication:** TypeScript explicitly aligns with dual-format prompt usage for nomic embeddings; Rust currently uses a single raw-text path.

### 4.3 Model metadata and dimension handling

| Area | TypeScript | Rust | Discrepancy |
|---|---|---|---|
| Stored model metadata | stores model ID from embedder | storage supports model name, but `CodeGraph::store_embedding` parameter naming is ambiguous (`text`) | Rust API contract is easier to misuse |
| Dimension config | fixed constant `EMBEDDING_DIMENSION` (768) | configurable dimension in `EmbedderConfig` (default 768) | Rust is more flexible, but requires stronger output-shape validation |

### 4.4 Operational parity note

TypeScript embedding code currently exposes richer task-level APIs (`embed`, `embedQuery`, typed batch mode). Rust currently exposes a single embedding path and uses it for search queries.

**Gap:** Rust should add first-class task-specific embedding methods to match intended retrieval behavior.

---

## 5) Productization/installer parity

TypeScript repository includes installer modules and templates for setup UX (`src/installer/*`) and project bootstrap behavior.

Rust workspace currently focuses on core crates + CLI/MCP implementation; equivalent installer UX layer is not represented as a dedicated crate/module set at parity with TS installer flow.

**Gap:** TS currently has richer first-run onboarding implementation in-tree.

---

## Suggested parity roadmap (short)

1. Add MCP compatibility layer accepting both TS-era and Rust-era argument names.
2. Port TS framework resolver registry patterns incrementally (start with top 5 frameworks by usage).
3. Add explicit Rust embedding APIs for `query` and `document` modes, with stable prompt templates and output-shape validation.
4. Decide target language matrix and either:
   - add missing Rust configs (C/C++/Kotlin/Liquid strategy), or
   - formally deprecate in TS to align supported matrix.
5. Define Rust-side installer strategy (retain TS installer, or port installer into Rust CLI with same UX contract).

