# Embedding Enrichment Design

> **Status**: Approved
> **Created**: 2026-02-04
> **Last Updated**: 2026-02-04
> **Approved**: 2026-02-04

## Overview

Enrich CodeGraph's semantic search embeddings with richer context from:
1. Existing extracted fields (decorators, type params, modifiers)
2. LSP queries (inferred types, resolved imports)
3. Graph relationships (callers, callees, siblings)
4. New extraction (comments, thrown errors, code snippets)
5. Module/package membership

## Priority Order

1. **Low-hanging fruit** - Use existing extracted fields in embeddings
2. **LSP enrichment** - Query language servers for inferred types and resolved imports
3. **Graph-aware** - Include callers/callees/siblings in embeddings
4. **New extraction** - Extract comments, thrown errors, interfaces, code snippets
5. **Module/package membership** - Semantic package names over raw file paths

---

## Design Decisions

### Phase 1: Architecture

| ID | Decision | Choice | Why | Alternatives Considered |
|----|----------|--------|-----|------------------------|
| A1 | **Enrichment timing** | Separate phases after extraction | Keeps extraction fast/deterministic; LSP optional; phases can run independently | Inline enrichment: simpler but couples extraction to LSP availability |
| A2 | **Phase order** | Extract → LSP → Embed (graph computed inline) | LSP needs extracted nodes; Graph context computed during embedding from edges table (B2); no separate graph phase | Different orderings cause missing dependencies |

### Phase 2: LSP Integration

| ID | Decision | Choice | Why | Alternatives Considered |
|----|----------|--------|-----|------------------------|
| L1 | **Protocol** | Standard LSP over stdio | Consistency across TS/Dart/Rust; one trait implementation; well-documented; Rust LSP client libraries available | Raw tsserver: faster but proprietary protocol requiring custom implementation |
| L2 | **TypeScript server** | `typescript-language-server` | Standard LSP protocol; same pattern as Dart/Rust; maintained by community | Raw tsserver: 5-10ms faster per request but requires custom JSON protocol handler |
| L3 | **Supported languages** | TypeScript, Dart, Rust (extensible) | Cover primary use cases; all have mature LSP servers | Start with one: slower rollout; All languages: scope creep |
| L4 | **Enrichment scope** | Configurable: hybrid (default), comprehensive, or selective | Hybrid = comprehensive on `index`, selective on `sync`; initial index can run overnight for quality; sync must be fast for daily use | Comprehensive always: 5+ min on 10K nodes, too slow for sync; Selective always: misses alias resolution on initial index |
| L4a | **Selective scope definition** | Query nodes in changed files + nodes with new/changed imports + nodes missing LSP data | "Selective" must catch new imports added to existing functions, not just nodes missing all type info | Missing types only: misses new imports; Changed files only: misses dependents |
| L5 | **Server lifecycle** | Lazy spawn, keep alive during indexing, shutdown after | Amortize 2-5s startup cost; one server per language | Per-file spawn: simpler but 2-5s overhead per file |

### Phase 3: Incremental Updates

| ID | Decision | Choice | Why | Alternatives Considered |
|----|----------|--------|-----|------------------------|
| I1 | **Update strategy** | Change-aware with dependency tracking | Full re-enrichment too slow (hours on large repos); must track what depends on what | Full re-run: simpler but unusable for daily workflow |
| I2 | **Dependency tracking** | New `enrichment_deps` table | Know which nodes to re-enrich when a type definition changes | No tracking: re-enrich everything or nothing |
| I3 | **Cascade depth** | Configurable, default=1 | Direct importers usually sufficient; deeper cascades expensive; users can tune | Fixed depth: inflexible for different repo structures |
| I3a | **Dependency granularity** | File-level only | Simpler to implement and query; if a file changes, re-enrich all its dependents; fast enough for incremental use | Symbol-level: more precise but complex to populate; marginal benefit |
| I4 | **Enrichment execution** | Blocking with max concurrency | Predictable behavior ("index done = search ready"); parallel file processing minimizes wall-clock time | Background async: faster perceived speed but embeddings incomplete; User choice: adds complexity |
| I5 | **LSP concurrency model** | Configurable: single LSP (default) or multiple instances | Single LSP handles parallel files well; multiple instances for power users with large repos and memory to spare | Single only: leaves performance on table for big repos; Multiple only: wastes memory on small repos |
| I5a | **Multi-instance work distribution** | Work-stealing queue | Self-balancing; handles uneven file sizes; instances stay busy; simple to implement with channel/queue | Round-robin: uneven if file sizes vary; Directory partition: uneven if dirs vary; Package partition: uneven if packages vary |
| I6 | **LSP spawn failure** | Retry 3x with backoff, then fail (default) or degrade (configurable) | Handles transient failures; fail-fast default forces user to fix setup; degrade option for CI/environments without LSP | No retry: fails on transient issues; Always degrade: user may not notice reduced quality |
| I7 | **Per-file LSP errors** | Skip file, log warning, continue; server crash restarts server | One file with syntax error shouldn't block entire index; restart handles transient server issues | Fail entire index: too brittle; Retry indefinitely: hangs on persistent errors |
| I8 | **LSP query timeout** | 5 seconds fixed | Simple, predictable; if query takes >5s something is wrong with file or server | Adaptive: complexity for marginal benefit; Per-operation: over-engineering; No timeout: risks hangs |
| I9 | **LSP position encoding** | Convert to UTF-16 code units per LSP spec | LSP uses UTF-16, not bytes or chars; tree-sitter gives byte offsets; must convert for multi-byte characters (emoji, unicode) | Assume ASCII: breaks on unicode; Use bytes: wrong positions |
| I10 | **LSP workspace init** | Wait for project analysis with timeout (60s default), then query | tsserver/rust-analyzer need to build project model before accurate queries; spawn time (2-5s) ≠ ready time (30-60s on large projects) | Query immediately: inaccurate results; No timeout: hangs on broken projects |
| I11 | **Edge change detection** | Snapshot edges before extraction, diff after, compute affected nodes | Need to know which nodes had callers/callees change; extraction updates edges but doesn't track changes | No detection: miss re-embedding; Track in extraction: couples concerns |
| I12 | **Concurrent modification** | Lock files during extraction via `.codegraph/index.lock`; detect mtime changes; re-extract if modified | Full index takes hours; files may change mid-index; race conditions cause stale data | No locking: race conditions; Per-file locks: complex |
| I13 | **Full re-embed triggers** | Schema migration, config change, `--force-reembed` flag | Incremental workflow handles file changes; need explicit trigger for global state changes (new columns, budget change, LSP enabled) | Auto-detect: complex version tracking; Manual only: users forget |

### Phase 4: Graph Context

| ID | Decision | Choice | Why | Alternatives Considered |
|----|----------|--------|-----|------------------------|
| G1 | **Graph context fields** | callees, callers, siblings, implements, extends | Cover call graph, inheritance, and sibling relationships; children omitted (methods already reference parent via file_path) | Fewer fields: less context; More fields: diminishing returns; Children: redundant with containment |
| G2 | **List size limits** | callees=10, callers=5, siblings=8 | Prevent embedding bloat; prioritize most relevant | Unlimited: embedding quality degrades with noise |
| G3 | **Truncation priority** | Hybrid: decorated first → call frequency → alphabetical | Decorated functions highest search value; frequency adds relevance; alphabetical ensures determinism | Alphabetical only: no semantic preference; Frequency only: misses decorated nodes; Single criterion: loses signal |
| G4 | **Transitive relationships** | Direct only | Transitive relationships explode quickly and add noise; direct provides strong context; users can navigate graph for deeper connections | One level: larger embeddings; Configurable: complexity; Smart transitive: unpredictable |
| G5 | **Cycle handling** | No special handling | Direct-only relationships (G4) means cycles don't cause issues; embedding reflects reality; no infinite loops possible | Mark cycles: adds complexity for minimal search value; Break cycles: loses information |
| G6 | **Call frequency tracking** | Count at query time | Call counts only used during enrichment; graph enrichment already queries all edges so counting is cheap; no schema change needed | Track in schema: overkill for enrichment-only use; Skip frequency: loses relevance signal |

### Phase 5: Embedding Content

| ID | Decision | Choice | Why | Alternatives Considered |
|----|----------|--------|-----|------------------------|
| E1 | **Field ordering** | Decorators first | Decorators carry domain labels (@Controller, @Test, #[derive]) that directly match search terms; embedding models weight earlier content more heavily | Signature first: core identity but less searchable; Docstring first: natural language but often missing; Keep current: loses semantic priority |
| E2 | **Decorator formatting** | Raw format | Decorator syntax (@Controller, #[derive]) is what users search for; labels/expansion creates indirection that may hurt matching | Labeled: adds noise; Expanded: loses original syntax; Hybrid: inconsistent |
| E3 | **Modifier formatting** | Code-like order | Matches how developers write and search (`public static async`); natural match for code searches | Space-separated: arbitrary order; Labeled: adds noise; Natural language: doesn't match code searches |
| E4 | **Code snippet length** | Adaptive: full body up to 50 lines | Small functions get full code; large ones get truncated; maximizes signal without bloating embeddings | Fixed 20: misses context in larger functions; Fixed 50: wastes space on small functions; No snippet: loses implementation semantics |
| E5 | **Comment extraction** | Docstring separate, inline in snippet | Docstrings are structured (@param, @returns); inline comments capture implementation notes in context; clean separation of concerns | All inline: duplicates snippet content; TODO only: misses descriptive comments; No extraction: loses docstring structure |
| E6 | **Test association** | Hybrid: naming/directory convention + import analysis fallback | Conventions handle 80% automatically (foo.test.ts, __tests__/); import analysis catches edge cases; no user action required | Convention only: misses non-standard layouts; Imports only: expensive for simple cases; Explicit annotation: requires user effort |
| E6a | **Test convention patterns** | Language-specific: TS/JS (`.test.ts`, `.spec.ts`, `__tests__/`), Go (`_test.go`), Python (`test_*.py`, `*_test.py`), Rust (`#[cfg(test)]`, `tests/`), Java (`*Test.java`, `src/test/`) | Different languages have different conventions; must support all | Single pattern: misses most tests; User config only: bad defaults |
| E7 | **Test data in embedding** | Test names only | Test names are written to be descriptive ("should handle expired cards"); compact and searchable | Names + descriptions: descriptions often absent; Names + assertions: adds noise; File reference only: loses descriptive value |
| E8 | **Thrown error extraction** | Both: parse throw statements + type annotations | Annotations give declared errors; throw parsing catches actual usage; both valuable for search ("auth errors", "validation failures") | Throw only: misses declared; Annotations only: misses actual; Skip: loses error semantics |

### Phase 6: Module/Package Membership

| ID | Decision | Choice | Why | Alternatives Considered |
|----|----------|--------|-----|------------------------|
| M1 | **Package detection** | Manifest → workspace → directory heuristics | Manifest most accurate; workspace detection handles mono-repos; directory heuristics catch edge cases | Manifest only: misses non-standard setups; Directory only: less semantic |
| M2 | **No manifest fallback** | Directory path from repo root | Always derivable; gives useful context; `src/services/payment` more semantic than full file path | File path only: redundant; Parent dir only: may collide; Empty: loses context |
| M3 | **Mono-repo package format** | Preserve manifest exactly | Respects user intent; matches how they reference packages; `@myapp/payments` if written, `payments` if written | Scope + name: may not match manifest; Path: loses semantic name; Just name: loses scope |

### Phase 7: Configuration

| ID | Decision | Choice | Why | Alternatives Considered |
|----|----------|--------|-----|------------------------|
| C1 | **Config format** | JSON | Matches legacy TypeScript version; users with existing `.codegraph/config.json` won't need migration; consistent with existing codebase | TOML: Rust standard but requires migration; Both: complexity |

### Phase 8: Embedding Budget

| ID | Decision | Choice | Why | Alternatives Considered |
|----|----------|--------|-----|------------------------|
| B1 | **Token budget** | Configurable, default 2,000 tokens (~8,000 chars) | Different codebases have different needs; 2,000 is well within nomic's quality zone; leaves headroom below 8,192 limit | Fixed 2,000: inflexible; 4,000: may degrade quality; 1,500: too aggressive |
| B2 | **Graph context storage** | Compute at embedding time, don't denormalize | Avoids staleness when edges change; graph queries are fast with indexes; G4 (direct only) and G2 (limits) keep queries bounded; consistency > small perf gain | Denormalize: faster embedding but stale on edge changes; Edge-triggered invalidation: complex; Eventual consistency: confusing behavior |
| B3 | **Truncation priority** | Tiered: never cut identity fields; proportional cut content; aggressive cut graph lists | Identity (decorators, name, signature) critical for search; docstring/snippet can be reduced; graph context is supplementary | Reverse order: loses important fields; Smart/largest first: unpredictable; Snippet first: may over-truncate |
| B4 | **Empty field handling** | Conditional inclusion - omit empty fields | Keeps embedding compact; avoids noise from `resolves to:` on functions or `throws:` on classes; template shows superset of possible fields | Node-type templates: harder to maintain; Include empty: adds noise |
| B5 | **Token counting method** | Use tiktoken cl100k_base as proxy; validate against nomic tokenizer periodically | Need deterministic counting without calling embedding API; cl100k_base is close enough; periodic validation catches drift | Actual nomic tokenizer: requires API call per node; char/4: too imprecise |
| B6 | **Tier 1 budget overflow** | If Tier 1 exceeds 80% of budget, truncate signature (keep first 200 chars), then truncate decorators (keep first 10) | Edge case: 50 decorators or 500-char signature; must have fallback | Fail: breaks embedding; No limit: quality degrades |
| B7 | **Graph query batching** | Batch graph context queries per 1000 nodes using CTEs | 100K nodes × 3 queries = 300K queries; batch with single CTE query per batch | Per-node queries: too slow at scale; Single mega-query: memory pressure |
| B8 | **Git activity truncation boost** | Optional: use git signals to prioritize what to keep | When truncating, prefer content from active files; priority order: last_modified (most recent wins) > churn (lines changed) > commit_count; default off | No boost: loses relevance signal; Always on: adds git query overhead |

### Phase 9: Quality Measurement

| ID | Decision | Choice | Why | Alternatives Considered |
|----|----------|--------|-----|------------------------|
| Q1 | **Quality strategy** | Golden queries + A/B comparison + CI integration | Golden queries: validate end-to-end; A/B: debug which fields help/hurt; CI: catch regressions; different tools for different purposes | Single approach: incomplete coverage |

---

## Quality Measurement Strategy

### Extend Existing Evaluation Framework

The existing `__tests__/evaluation/` framework provides:
- Test cases with queries, expected symbols, min recall/precision
- Ground truth call graphs
- Precision/recall/F1 scoring

**Extensions needed:**

1. **A/B comparison mode** - Run same queries with two embedding configurations:
   ```typescript
   interface ABTestResult {
     testCaseId: string;
     baselineRecall: number;
     enrichedRecall: number;
     recallDelta: number;
     baselinePrecision: number;
     enrichedPrecision: number;
     precisionDelta: number;
   }
   ```

2. **Enrichment-specific test cases** - Queries that target enriched fields:
   ```typescript
   // Decorator search
   { query: "API endpoint users", expectedSymbols: ["UsersController"] } // @Controller('/users')

   // Test name search
   { query: "expired card handling", expectedSymbols: ["processPayment"] } // tested by: "should handle expired cards"

   // Error type search
   { query: "authentication failures", expectedSymbols: ["login"] } // throws: AuthenticationError

   // Inferred type search
   { query: "async user fetch", expectedSymbols: ["getUsers"] } // type: Promise<User[]>
   ```

3. **Baseline recording** - Capture pre-enrichment metrics:
   ```bash
   codegraph eval --save-baseline baseline-v1.json
   codegraph eval --compare baseline-v1.json  # Shows delta
   ```

### Success Criteria

| Metric | Minimum Improvement | Notes |
|--------|---------------------|-------|
| Search recall@10 | +5% | More relevant results in top 10 |
| Search precision@10 | No regression | Don't add noise |
| Decorator queries | +20% recall | These should work much better |
| Context F1 | +10% | Better context for AI tasks |

### CI Integration

- Run evaluation on every PR that touches embedding code
- Fail PR if recall drops >2% or precision drops >5%
- Store metrics in CI artifacts for trend analysis

---

## Architecture

### Enrichment Pipeline

```
Phase 1: Extraction (existing)
    Tree-sitter walk → Nodes + Edges → SQLite

Phase 2: LSP Enrichment (new)
    For each language with LSP support:
        1. Spawn language server
        2. Wait for workspace initialization (I10: up to 60s timeout)
           - tsserver: wait for project load
           - rust-analyzer: wait for cargo metadata
        3. Convert positions to UTF-16 code units (I9)
        4. Query hover/definition for nodes (scope per L4/L4a)
        5. Update nodes with inferred_type, resolved_import_path
        6. Populate enrichment_deps from definition responses

Phase 3: Embedding Text Generation (modified)
    Batch nodes in groups of 1000 (B7):
        1. Load node fields (decorators, signature, etc.)
        2. Batch query edges table for graph context via CTE
        3. For each node:
           a. Count tokens using tiktoken cl100k_base (B5)
           b. Apply truncation: Tier 1 with overflow protection (B6)
           c. Apply truncation: Tiers 2-3 as needed (B3)
           d. Combine into embedding text (decorators first)
    → createNodeText() output

Phase 4: Vector Generation (existing)
    Embed enriched text → Store in vectors table
```

**Note:** Graph context is computed inline during embedding generation (not stored).
This ensures consistency when edges change without requiring a separate enrichment phase.

### Full Re-embed Triggers (I13)

The following trigger a complete re-embedding of all nodes:
- Schema migration (new columns added to nodes table)
- Config change affecting embeddings (token budget, field limits)
- First time enabling LSP enrichment
- `codegraph index --force-reembed` flag
- Embedding model change

Detection: Store `embedding_version` in metadata table. Bump on any of the above.

### LSP Abstraction Layer

```
                    ┌─────────────────────────┐
                    │   LspEnricher trait     │
                    │   ─────────────────     │
                    │   start()               │
                    │   hover(file, pos)      │
                    │   definition(file, pos) │
                    │   shutdown()            │
                    └───────────┬─────────────┘
                                │
            ┌───────────────────┼───────────────────┐
            │                   │                   │
    ┌───────▼───────┐   ┌───────▼───────┐   ┌───────▼───────┐
    │  TypeScript   │   │  Dart         │   │  Rust         │
    │  Enricher     │   │  Enricher     │   │  Enricher     │
    └───────────────┘   └───────────────┘   └───────────────┘
```

### Supported Language Servers

| Language | Server | Install | Spawn |
|----------|--------|---------|-------|
| TypeScript/JavaScript | `typescript-language-server` | `npm i -g typescript-language-server` | `typescript-language-server --stdio` |
| Dart/Flutter | Dart Analysis Server | Bundled with Dart SDK | `dart language-server --protocol=lsp` |
| Rust | rust-analyzer | `rustup component add rust-analyzer` | `rust-analyzer` |

---

## Database Schema Changes

```sql
-- LSP enrichment fields
ALTER TABLE nodes ADD COLUMN inferred_type TEXT;
ALTER TABLE nodes ADD COLUMN resolved_import_path TEXT;

-- New extraction fields
ALTER TABLE nodes ADD COLUMN code_snippet TEXT;  -- First N lines of body (adaptive up to 50)
ALTER TABLE nodes ADD COLUMN thrown_errors TEXT; -- JSON array of error type names
ALTER TABLE nodes ADD COLUMN test_names TEXT;    -- JSON array of associated test names
ALTER TABLE nodes ADD COLUMN package_name TEXT;  -- Semantic package name (from manifest or dir path)

-- Dependency tracking for incremental LSP updates (file-level granularity)
CREATE TABLE enrichment_deps (
    node_id TEXT,
    depends_on_file TEXT,
    PRIMARY KEY (node_id, depends_on_file)
);
-- Populated during LSP enrichment from definition responses
-- When depends_on_file changes, re-enrich all nodes that depend on it

-- NOTE: Graph context (callees, callers, siblings, implements, extends, children)
-- is NOT stored in nodes table. It's computed at embedding time by querying
-- the edges table. This avoids staleness when edges change. (Decision B2)

-- Metadata for version tracking and full re-embed detection (I13)
CREATE TABLE IF NOT EXISTS metadata (
    key TEXT PRIMARY KEY,
    value TEXT
);
-- Keys: embedding_version, schema_version, config_hash, last_full_embed
```

---

## Embedding Template

Ordering rationale: Decorators first because they carry domain labels that directly match search terms. Embedding models weight earlier content more heavily.

**Note:** This template shows all *possible* fields. Empty fields are omitted (B4). For example, a function won't have `resolves to:`, and an import won't have `throws:`.

```
{decorators}                      # if present
{kind} {name} in {file_path}      # always
{modifiers}                       # if present (public static async)
<{type_parameters}>               # if present
{signature}                       # if present
type: {inferred_type}             # if LSP provided inferred type
implements: {interfaces}          # classes/interfaces only
extends: {parent_class}           # classes only
calls: {callee_names}             # functions/methods only
called by: {caller_names}         # functions/methods only
throws: {thrown_errors}           # functions/methods only
package: {package_name}           # always (from manifest or dir path)
resolves to: {resolved_import_path}  # imports only
tested by: {test_names}           # if test association found
siblings: {sibling_names}         # if has siblings in same container
---
{docstring}                       # if present
{code_snippet}                    # functions/methods only, up to 50 lines
```

### Truncation Tiers (when over token budget)

| Tier | Fields | Truncation Behavior |
|------|--------|---------------------|
| **1 - Identity** | decorators, kind, name, file_path, modifiers, type_parameters, signature | Normally included in full; overflow protection if >80% budget (B6) |
| **2 - Content** | docstring, code_snippet | Truncate to fit remaining budget; prefer keeping first N lines |
| **3 - Context** | inferred_type, implements, extends, calls, called_by, throws, package, resolves_to, tested_by, siblings | Reduce list sizes or omit entirely if needed |

**Algorithm:**
1. Calculate Tier 1 size using tiktoken cl100k_base (B5)
2. **Overflow protection (B6):** If Tier 1 > 80% of budget:
   - Truncate signature to first 200 chars
   - If still over, keep only first 10 decorators
   - Log warning for manual review
3. Calculate remaining budget after Tier 1
4. If Tier 3 fields fit, include them; otherwise reduce/omit
5. Allocate remaining budget to Tier 2 fields proportionally
6. **Optional git boost (B8):** If `git_activity_boost` enabled, when truncating prefer content from active files. Priority: last_modified (most recent wins) > churn (lines changed) > commit_count. Requires git history query per file batch.

---

## Incremental Update Workflow

```
File Changed
    │
    ├──► Acquire lock (.codegraph/index.lock)
    │
    ├──► Snapshot edges for changed files (I11)
    │    SELECT * FROM edges WHERE source_file IN (<changed_files>) OR target_file IN (<changed_files>)
    │
    ├──► Re-extract AST (tree-sitter)
    │    Creates/updates nodes and edges
    │    Check mtime; re-extract if file modified during extraction (I12)
    │
    ├──► Diff edges: compute added/removed (I11)
    │    affected_nodes = nodes involved in added or removed edges
    │
    ├──► Find LSP dependents
    │    SELECT node_id FROM enrichment_deps WHERE depends_on_file IN (<changed_files>)
    │
    ├──► Re-query LSP (selective scope per L4a)
    │    - Nodes in changed files
    │    - Nodes with new/changed imports (from edge diff)
    │    - Nodes missing LSP data
    │    - LSP-dependent nodes
    │    Update enrichment_deps with new dependencies discovered
    │
    ├──► Re-embed affected nodes
    │    - Nodes in changed files
    │    - Nodes whose LSP data changed
    │    - Nodes from edge diff (callers/callees affected)
    │    Graph context computed fresh via batched CTEs (B7)
    │
    └──► Release lock
```

### How enrichment_deps is Populated

During LSP enrichment, when we query `textDocument/definition` for an import or type reference:

```
Node: src/orders/service.ts:OrderService.createOrder
Query: definition of `User` type
Response: file:///src/types/user.ts

→ INSERT INTO enrichment_deps (node_id, depends_on_file)
  VALUES ('OrderService.createOrder', 'src/types/user.ts')
```

Later, when `src/types/user.ts` changes, we find all nodes depending on it and re-enrich them.

**Edge change handling:** When edges change (new call added, import removed), any node
involved in that edge needs re-embedding. The graph context is recomputed from the
edges table at embedding time, so it's always fresh. No separate invalidation needed.

### Test Convention Patterns by Language (E6a)

| Language | File Patterns | Directory Patterns | Notes |
|----------|--------------|-------------------|-------|
| TypeScript/JavaScript | `*.test.ts`, `*.spec.ts`, `*.test.js`, `*.spec.js` | `__tests__/`, `test/`, `tests/` | Check jest/vitest config for custom patterns |
| Go | `*_test.go` | (same directory as source) | Tests must be in same package |
| Python | `test_*.py`, `*_test.py` | `tests/`, `test/` | pytest conventions |
| Rust | (inline `#[cfg(test)]` modules) | `tests/` (integration tests) | Unit tests are in same file |
| Java/Kotlin | `*Test.java`, `*Tests.java`, `*Spec.kt` | `src/test/java/`, `src/test/kotlin/` | Maven/Gradle conventions |
| C#/.NET | `*Tests.cs`, `*Test.cs` | `*.Tests/` project | Separate test projects |
| Ruby | `*_spec.rb`, `*_test.rb` | `spec/`, `test/` | RSpec vs Minitest |
| Swift | `*Tests.swift` | `Tests/` | Swift Package Manager |
| Dart | `*_test.dart` | `test/` | Flutter conventions |

---

## Configuration

```json
// .codegraph/config.json (extends existing format)
{
  "version": 2,
  "lsp": {
    "enabled": true,
    "typescript": {
      "enabled": true,
      "server": "typescript-language-server",
      "args": ["--stdio"]
    },
    "dart": {
      "enabled": true,
      "server": "dart",
      "args": ["language-server", "--protocol=lsp"]
    },
    "rust": {
      "enabled": true,
      "server": "rust-analyzer",
      "args": []
    }
  },
  "enrichment": {
    "lsp_scope": "hybrid",  // "hybrid" (comprehensive on index, selective on sync), "comprehensive", or "selective"
    "cascade_depth": 1,
    "lsp_instances": 1,
    "on_lsp_unavailable": "fail",
    "query_timeout_secs": 5,
    "workspace_init_timeout_secs": 60  // Wait for LSP project analysis (I10)
  },
  "embedding": {
    "max_tokens": 2000,
    "max_callees": 10,
    "max_callers": 5,
    "max_siblings": 8,
    "max_snippet_lines": 50,
    "git_activity_boost": false  // Optional: use git signals for truncation priority
  }
}
```

---

## Open Questions

*All questions resolved.*

---

## Conflict Review Log

*Record conflicts found during phase reviews and their resolutions here.*

### Review 1: After Phase 1 (Low-Hanging Fruit)

**Status**: Resolved

**Conflicts Found**:
1. Duplicate decision IDs (E3, E4 appeared twice) → Fixed: renumbered to E3-E6
2. Architecture text said "signature + decorators" but decision was decorators-first → Fixed: updated to reflect correct order

**No unresolved conflicts.**

### Review 2: After Phase 2 (LSP Integration)

**Status**: Resolved

**Conflicts Found**:
1. Open Questions still listed "Background enrichment (async vs blocking)" but I4 resolved this → Fixed: removed from Open Questions

**No unresolved conflicts.**

### Review 3: After Phase 3 (Graph-Aware Enrichment)

**Status**: Resolved

**Conflicts Found**:
1. G3 referenced "call frequency" but tracking method was undefined → Resolved: added G6 to count at query time

**No unresolved conflicts.**

### Review 4: After Phase 4 (New Extraction)

**Status**: Resolved

**Conflicts Found**:
1. Template said `{test_descriptions}` but E7 decided "test names only" → Fixed: changed to `{test_names}`
2. Template had `{inline_comments}` but E5 decided inline comments are in snippet → Fixed: removed separate line

**No unresolved conflicts.**

### Review 5: After Phase 5 (Module/Package Membership)

**Status**: Resolved

**Conflicts Found**:
1. Template said `{module_name}` but schema uses `package_name` → Fixed: changed template to `{package_name}`

**No unresolved conflicts.**

### Review 6: Final Comprehensive Review

**Status**: Resolved

**Checks Performed**:
1. Decision IDs unique and sequential ✓
2. All template fields have schema columns ✓
3. Config options match decisions ✓
4. Architecture section aligns with decisions ✓
5. No logical contradictions ✓

**Conflicts Found**:
1. Architecture said "comments" but E5 decided docstring separate, inline in snippet → Fixed: changed to "docstring"
2. Config used TOML but legacy codebase uses JSON → Fixed: changed to JSON format, added C1 decision
3. No embedding token budget specified → Added B1-B3 decisions for budget and truncation
4. Graph context denormalization would cause staleness on edge changes → Changed to compute at embedding time (B2), removed graph columns from schema, updated architecture
5. L4 comprehensive scope impractical at scale (10K nodes × 30ms = 5+ min) → Changed to configurable with hybrid default (comprehensive on index, selective on sync)
6. No quality measurement strategy → Added Q1 decision and full quality measurement section extending existing `__tests__/evaluation/` framework
7. `enrichment_deps.depends_on_symbol` undefined → Removed; simplified to file-level granularity only (I3a); documented how table is populated
8. I5 multi-instance work distribution undefined → Added I5a: work-stealing queue
9. Template has fields that only apply to certain node types (e.g., `resolves to:` for imports) → Added B4: conditional inclusion, omit empty fields

**No unresolved conflicts.**

### Review 7: Adversarial Review

**Status**: Resolved

**Issues Found and Resolutions:**

| # | Issue | Resolution |
|---|-------|------------|
| 1 | LSP position calculation uses UTF-16 but tree-sitter gives bytes | Added I9: convert to UTF-16 code units per LSP spec |
| 2 | Edge change detection missing from incremental workflow | Added I11: snapshot edges before, diff after, compute affected nodes; updated workflow |
| 3 | Tier 1 "never cut" can exceed budget (50 decorators, 500-char signature) | Added B6: overflow protection - truncate signature to 200 chars, limit to 10 decorators |
| 4 | Token counting method unspecified | Added B5: use tiktoken cl100k_base as proxy, validate periodically |
| 5 | Full re-embed trigger missing for schema/config changes | Added I13: schema migration, config change, `--force-reembed` flag; version tracking |
| 6 | LSP workspace init time understated (project analysis takes 30-60s) | Added I10: wait for project analysis with 60s timeout |
| 7 | Selective sync misses new imports in existing functions | Added L4a: selective scope includes nodes with new/changed imports |
| 8 | Test convention patterns incomplete (Go, Python, Rust, etc.) | Added E6a: language-specific convention table |
| 9 | Graph query performance at scale (300K queries for 100K nodes) | Added B7: batch queries per 1000 nodes using CTEs |
| 10 | Concurrent modification during indexing causes race conditions | Added I12: file locking, mtime change detection |

**No unresolved conflicts.**

### Review 8: Final Conflict Detection

**Status**: Resolved

**Checks Performed**:
1. Decision IDs unique and sequential ✓
2. All template fields have data sources ✓
3. Config options match decisions ✓
4. Architecture aligns with decisions ✓
5. Cross-references valid ✓
6. Terminology consistent ✓

**Issues Found and Resolutions:**

| # | Issue | Resolution |
|---|-------|------------|
| 1 | G1 listed "children" but template had no `children:` line | Removed "children" from G1; children redundant since methods already reference parent via file_path |
| 2 | G2 missing limit for "children" | N/A after removing children from G1 |
| 3 | A2 said "Graph" as separate phase but B2 shows inline computation | Clarified A2: "Extract → LSP → Embed (graph computed inline)" |

**No unresolved conflicts.**

---

## Decision Summary

Total decisions: **47**

| Phase | Count | IDs |
|-------|-------|-----|
| Architecture | 2 | A1-A2 |
| LSP Integration | 6 | L1-L5, L4a |
| Incremental Updates | 15 | I1-I13, I3a, I5a |
| Graph Context | 6 | G1-G6 |
| Embedding Content | 9 | E1-E8, E6a |
| Module/Package | 3 | M1-M3 |
| Configuration | 1 | C1 |
| Embedding Budget | 8 | B1-B8 |
| Quality Measurement | 1 | Q1 |

---

## Implementation Plan

*To be created after design is approved.*
