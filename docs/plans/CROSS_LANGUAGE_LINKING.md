# Cross-Language AppSync Linking (Monorepo Approach)

**Date:** 2026-02-28
**Status:** Draft
**Crates affected:** `codegraph-types`, `codegraph-db`, `codegraph-extraction`, `codegraph-resolution`, `codegraph-core`, `codegraph-mcp`, `codegraph-cli`
**Depends on:** None (standalone feature)

## Context

The original LINKED_REPOS.md plan proposed a workspace system (`~/.codegraph-workspaces/`, separate `cross_repo_edges` table, repo identity, etc.) to link TypeScript backend AppSync resolvers with Dart/Flutter frontend GraphQL calls.

After analysis with Codex, we identified a simpler approach: treat sibling repos as a single project by running `codegraph init` on the parent folder. This eliminates the need for workspace infrastructure entirely — file paths naturally scope by subfolder, existing graph traversal works unchanged, and no new tables are needed.

**Key decisions:**
- **Monorepo-style**: Plain parent folder (non-git) containing child git repos
- **Rust only**: TS pipeline lacks Dart support
- **Design seams**: Architect for future workspace support without building it now
- **Child repo hooks**: Install git hooks in each child `.git` that trigger `codegraph sync <parent>`
- **Separate edge kinds**: Use explicit `AppSyncQueryCall`, `AppSyncMutationCall`, `AppSyncSubscription` edge kinds (not a generic `ResolvesTo`) — matches the codebase's kind-filter-first architecture
- **Language-agnostic embedded GraphQL**: One-pass candidate collection during `walk_ast()` then deferred GraphQL parsing. Separate `EmbeddedDslConfig` registry with per-language string node types, tag hints, and interpolation patterns. Hybrid parsing: tree-sitter-graphql + heuristic fallback for interpolated strings
- **Rules-based resolver linking**: Backend resolver identification is driven by a user-editable `.codegraph/resolver-rules.json` file with tree-sitter S-expression queries (`.scm` files) for AST pattern matching and capture-based name extraction. Not hardcoded to any specific framework — supports AppSync, NestJS, Spring GraphQL, etc. Same file also contains scoped `scans` config for frontend GraphQL detection.

**Folder structure:**
```
/products/vantage/           <- codegraph init here
  .codegraph/codegraph.db
  core-platform-backend/     <- TypeScript, has .git
  guest-vue-front-end/       <- Dart/Flutter, has .git
  shared-dart-lib/           <- Dart library, has .git
```

---

## Phase 1: Foundation Types & Schema Migration

**Goal**: Add new `EdgeKind` variants, `NodeKind::GraphQLOperation`, `Node.metadata` field, and schema migration v4.

### Files to modify

**`codegraph/crates/codegraph-types/src/lib.rs`**
- Add three `EdgeKind` variants (~line 230):
  - `AppSyncQueryCall` — frontend GraphQL query resolves to backend AppSync query handler
  - `AppSyncMutationCall` — frontend GraphQL mutation resolves to backend AppSync mutation handler
  - `AppSyncSubscription` — frontend GraphQL subscription resolves to backend subscription model
- Update `all()`, `as_str()`, `is_structural()` (none are structural), `FromStr`, `Display`
- Rationale: The codebase is kind-filter-first (all traversal/query APIs filter by `EdgeKind` arrays, no metadata-based edge queries exist, DB indexes are on `kind`). Separate kinds are type-safe, debuggable, and match existing patterns. Future API families (REST, gRPC) add their own kinds.
- Add `NodeKind::GraphQLOperation` variant (~line 56). Update `as_str()`, `FromStr`.
- Add `pub metadata: Option<serde_json::Value>` to `Node` struct (~line 642). Initialize as `None`.

**`codegraph/crates/codegraph-db/src/migrations.rs`**
- Add `migrate_to_v4()`: `ALTER TABLE nodes ADD COLUMN metadata TEXT`
- Update `run_migrations()` to call it (current version is v3)

**`codegraph/crates/codegraph-db/src/queries.rs`**
- Update `insert_node()` to include `metadata` column
- Update `update_node()` (~line 120) to include `metadata` column — without this, sync updates would silently drop metadata on re-extraction
- Update `row_to_node()` to read `metadata` column
- Add `get_nodes_with_metadata_key(conn, key)` — `WHERE metadata LIKE '%"key":%'`
  - **Note**: LIKE-based metadata lookup is adequate for V1 volume. If AppSync node counts grow large, consider SQLite generated columns (`ALTER TABLE nodes ADD COLUMN appsync_type TEXT GENERATED ALWAYS AS (json_extract(metadata, '$.appsync_type'))`) with an index for O(1) lookups.
- `get_nodes_by_kind(conn, NodeKind)` already exists (~line 260) — no changes needed
- Update `insert_edge()` (~line 499): currently uses `INSERT OR IGNORE` which silently drops metadata updates on re-index. For AppSync edges, use `INSERT ... ON CONFLICT(source_id, target_id, kind) DO UPDATE SET metadata = excluded.metadata` to preserve updated confidence/match data across re-indexing.

**`codegraph/crates/codegraph-db/src/lib.rs`**
- Export `migrate_to_v4`

### Test
```
cargo test -p codegraph-types -p codegraph-db
```

---

## Phase 2: Rules-Based Resolver Identification

**Goal**: Create a configurable rules engine that identifies backend GraphQL resolvers using tree-sitter AST queries and extracts resolver names into `node.metadata`.

**Key insight**: Different frameworks use different AST patterns to declare resolvers — AppSync uses `@AppSyncQuery({ methodName: "X" })`, NestJS uses `@Query('getUser')` on methods, Spring uses `@QueryMapping` with method name convention. Rather than hardcoding any specific framework or relying on string-matching heuristics, rules use **tree-sitter S-expression queries** to match AST structure and capture resolver names directly. This is language-specific by nature (AST node types differ per grammar), but maximally precise and extensible.

### Rules File Format

**`.codegraph/resolver-rules.json`**:
```json
{
  "version": 1,
  "scans": [
    {
      "name": "dart-frontend-queries",
      "file_patterns": ["**/queries/**", "**/graphql/**"],
      "languages": ["dart"],
      "tag_hints": [],
      "min_length": 20
    },
    {
      "name": "ts-gql-templates",
      "file_patterns": ["**/graphql/**", "**/hooks/**"],
      "languages": ["typescript", "tsx"],
      "tag_hints": ["gql"],
      "min_length": 20
    }
  ],
  "rules": [
    {
      "name": "appsync-query",
      "description": "AWS AppSync query handler",
      "languages": ["typescript", "tsx"],
      "file_pattern": "**/resolvers/**",
      "query_file": "queries/appsync-query.scm",
      "captures": {
        "resolver_name": "resolver_name",
        "node": "node"
      },
      "operation_type": "query",
      "edge_kind": "AppSyncQueryCall"
    },
    {
      "name": "appsync-mutation",
      "description": "AWS AppSync mutation handler",
      "languages": ["typescript", "tsx"],
      "file_pattern": "**/resolvers/**",
      "query_file": "queries/appsync-mutation.scm",
      "captures": {
        "resolver_name": "resolver_name",
        "node": "node"
      },
      "operation_type": "mutation",
      "edge_kind": "AppSyncMutationCall"
    },
    {
      "name": "appsync-subscription",
      "description": "AWS AppSync subscription model",
      "languages": ["typescript", "tsx"],
      "query_file": "queries/appsync-subscription.scm",
      "captures": {
        "resolver_name": "class_name",
        "node": "node"
      },
      "transforms": {
        "strip_suffix": "SubscriptionModel",
        "prepend": "subscribeTo"
      },
      "operation_type": "subscription",
      "edge_kind": "AppSyncSubscription"
    },
    {
      "name": "nestjs-query",
      "description": "NestJS GraphQL query resolver",
      "languages": ["typescript", "tsx"],
      "query_file": "queries/nestjs-query.scm",
      "captures": {
        "resolver_name": "resolver_name",
        "node": "node"
      },
      "operation_type": "query",
      "edge_kind": "AppSyncQueryCall"
    }
  ]
}
```

### Tree-Sitter Query Files

Queries live in `.codegraph/queries/` as `.scm` files. Named captures (`@resolver_name`, `@node`, etc.) handle both identification and extraction in one pass.

**`.codegraph/queries/appsync-query.scm`**:
```scheme
(class_declaration
  (decorator
    (call_expression
      function: (identifier) @_dec
      (#eq? @_dec "AppSyncQuery")
      arguments: (arguments
        (object
          (pair
            key: (property_identifier) @_key
            (#eq? @_key "methodName")
            value: (string
              (string_fragment) @resolver_name)))))) @node
```

**`.codegraph/queries/nestjs-query.scm`**:
```scheme
(method_definition
  (decorator
    (call_expression
      function: (identifier) @_dec
      (#eq? @_dec "Query")
      arguments: (arguments
        (string
          (string_fragment) @resolver_name)))) @node
```

**`.codegraph/queries/appsync-subscription.scm`**:
```scheme
(class_declaration
  name: (type_identifier) @class_name
  (decorator
    (call_expression
      function: (identifier) @_dec
      (#eq? @_dec "SubscriptionModel")))) @node
```

**`.codegraph/queries/spring-query-mapping.scm`** (example for Java):
```scheme
(method_declaration
  (modifiers
    (marker_annotation
      name: (identifier) @_dec
      (#eq? @_dec "QueryMapping")))
  name: (identifier) @resolver_name) @node
```

### Rule Configuration

| Field | Type | Description |
|-------|------|-------------|
| `name` | `string` | Unique rule identifier |
| `description` | `string?` | Human-readable description |
| `languages` | `string[]` | Languages this rule applies to (required — queries are grammar-specific) |
| `file_pattern` | `string?` | Restrict to files matching this glob (e.g. `**/resolvers/**`) |
| `query` | `string?` | Inline tree-sitter S-expression query |
| `query_file` | `string?` | Path to `.scm` file relative to `.codegraph/` |
| `captures.resolver_name` | `string` | Name of the query capture that holds the resolver name |
| `captures.node` | `string?` | Name of the query capture for the node to tag (default: outermost matched node) |
| `transforms` | `object?` | Optional name transforms applied to the captured resolver name |
| `operation_type` | `string` | `"query"` / `"mutation"` / `"subscription"` |
| `edge_kind` | `string` | Edge kind to create: `AppSyncQueryCall`, `AppSyncMutationCall`, `AppSyncSubscription` |

Either `query` or `query_file` must be provided. If both are present, `query_file` takes precedence.

### Scan Configuration

| Field | Type | Description |
|-------|------|-------------|
| `name` | `string` | Unique scan identifier |
| `file_patterns` | `string[]?` | Only scan matching files for embedded GraphQL. Omit to scan all files of listed languages |
| `exclude_file_patterns` | `string[]?` | Skip these even if they match (generated code, mocks) |
| `languages` | `string[]?` | Restrict which languages get the embedded GraphQL scan. Omit to scan all supported |
| `tag_hints` | `string[]?` | Tagged template identifiers that boost confidence (e.g. `["gql"]` for TS/JS) |
| `min_length` | `usize?` | Minimum string length to consider (default 20) |

If `scans` is omitted entirely, scan all files of all supported languages (zero-config discovery mode).

### Name Transforms

Transforms are applied to the captured resolver name in order. Used when the resolver name must be derived (e.g., subscription class name → operation name).

| Field | Type | Description |
|-------|------|-------------|
| `strip_suffix` | `string?` | Remove trailing text from the name |
| `strip_prefix` | `string?` | Remove leading text |
| `prepend` | `string?` | Add text before the name |
| `append` | `string?` | Add text after the name |
| `to_camel_case` | `bool?` | Convert to camelCase |

### Files to modify

**`codegraph/crates/codegraph-core/src/resolver_rules.rs`** (new file)
- Define `ResolverRulesConfig` struct (deserializable from JSON):
  ```rust
  pub struct ResolverRulesConfig {
      pub version: u32,
      pub scans: Option<Vec<ScanConfig>>,
      pub rules: Vec<ResolverRule>,
  }
  pub struct ScanConfig {
      pub name: String,
      pub file_patterns: Option<Vec<String>>,
      pub exclude_file_patterns: Option<Vec<String>>,
      pub languages: Option<Vec<String>>,
      pub tag_hints: Option<Vec<String>>,
      pub min_length: Option<usize>,
  }
  pub struct ResolverRule {
      pub name: String,
      pub description: Option<String>,
      pub languages: Vec<String>,
      pub file_pattern: Option<String>,
      pub query: Option<String>,
      pub query_file: Option<String>,
      pub captures: CaptureConfig,
      pub transforms: Option<NameTransforms>,
      pub operation_type: String,
      pub edge_kind: String,
  }
  pub struct CaptureConfig {
      pub resolver_name: String,
      pub node: Option<String>,
  }
  pub struct NameTransforms {
      pub strip_suffix: Option<String>,
      pub strip_prefix: Option<String>,
      pub prepend: Option<String>,
      pub append: Option<String>,
      pub to_camel_case: Option<bool>,
  }
  ```
- `load_resolver_rules(data_dir: &Path) -> Option<ResolverRulesConfig>` — loads `.codegraph/resolver-rules.json`, returns None if missing. Loads any referenced `.scm` query files from `.codegraph/`.
- `compile_rule_queries(rules: &[ResolverRule], data_dir: &Path) -> Vec<CompiledRule>` — for each rule, load query from `query_file` or inline `query`, compile into a `tree_sitter::Query` for the appropriate language grammar. Cache compiled queries for reuse across files.
- `run_rules_on_tree(tree: &Tree, source: &[u8], compiled_rules: &[CompiledRule]) -> Vec<ResolverMatch>` — execute compiled queries against a parsed tree using `tree_sitter::QueryCursor`. For each match, extract the `resolver_name` capture text, apply transforms, return match with node position and metadata.
- `apply_transforms(name: &str, transforms: &NameTransforms) -> String` — applies strip/prepend/append/camelCase transforms in order

**`codegraph/crates/codegraph-core/src/config.rs`**
- Add `pub resolver_rules: Option<ResolverRulesConfig>` to `CodeGraphConfig`
- In `CodeGraphConfig::load()`: attempt to load `resolver-rules.json` from `.codegraph/`

**`codegraph/crates/codegraph-core/src/codegraph.rs`**
- After extraction (in both `index_all` and `sync_with_options`): for files matching rule `languages` and `file_pattern`, re-use the already-parsed tree-sitter tree to run compiled resolver queries. Populate matching nodes' `node.metadata` with `{"resolver_type": "query"|"mutation"|"subscription", "resolver_name": "X", "rule_name": "appsync-query"}`
- **Optimization**: The tree is already parsed during extraction. The resolver rules pass runs queries against the same tree — no re-parsing needed. The tree-sitter `QueryCursor` is lightweight and incremental.
- This pass runs before resolution (Phase 4)

**`codegraph/crates/codegraph-core/src/lib.rs`**
- Add `pub mod resolver_rules;`

### Test
```
cargo test -p codegraph-core
```
- Test query compilation: load `.scm` file, compile for TypeScript grammar
- Test query matching: AppSync query `.scm` matches class with `@AppSyncQuery` decorator, captures `resolver_name`
- Test NestJS query: `@Query('getUser')` captures `"getUser"` as resolver name
- Test Spring QueryMapping: `@QueryMapping` captures method name as resolver name
- Test subscription transforms: captures `class_name` "PaymentAddedSubscriptionModel" → transforms to `"subscribeToPaymentAdded"`
- Test rule priority: first matching rule wins
- Test no match: file without matching AST patterns returns empty
- Test language scoping: TypeScript query not run against Python files
- Test file_pattern scoping: rule restricted to `**/resolvers/**` skips other paths
- Test query_file vs inline query: both paths produce same results
- Test invalid query: graceful error on malformed S-expression
- Integration: load rules from JSON + `.scm` files, run against extracted tree, verify `node.metadata` populated

---

## Phase 3: Language-Agnostic Embedded GraphQL Extraction

**Goal**: Extract GraphQL operations embedded in string literals across ALL supported languages. Create `GraphQLOperation` nodes with metadata listing resolver names.

**Key insight**: GraphQL can be embedded in string literals in any language — Dart triple-quoted strings, JS/TS tagged templates (`gql\`...\``), Python triple-quoted strings, Ruby heredocs, Go raw strings, etc. Rather than a Dart-specific pass, we build a language-agnostic embedded DSL detection pipeline that runs as a second pass after `walk_ast()`.

**Architecture**: One-pass collection, deferred parsing.
1. **During `walk_ast()`** — collect string literal candidates into a `Vec<CandidateString>` as they're encountered (zero extra traversal cost). Each candidate records the text content, position, and enclosing node ID (already available as `parent_id` in the recursive walk).
2. **After `walk_ast()` scope ends** (freeing `&mut self.parser`) — filter and parse collected candidates with tree-sitter-graphql. This avoids re-walking the tree, which matters when indexing 3+ frontends and a large backend.

**Parsing strategy**: Hybrid with language-agnostic pipeline.
1. **Collect** candidate string/template nodes using a separate `EmbeddedDslConfig` registry (not LanguageConfig — DSL detection needs richer config: string node types, interpolation markers, tag hints, thresholds)
2. **Prefilter** (cheap lexical): string length floor (>20 chars), must contain `{`, keyword hint (`query`/`mutation`/`subscription`/`fragment`), parse text dedupe by content hash. No per-file candidate cap — existing `DEFAULT_EXCLUDE_PATTERNS` (`*.min.js`, `*.bundle.js`, `*.chunk.js`) and `max_file_size` (10MB) already prevent pathological files from entering the pipeline. Log a warning if a single file produces >100 candidates as a diagnostic signal.
3. **Parse** using tree-sitter-graphql (already compiled by default via `all-languages` feature) for robust extraction of operation type, name, and top-level fields
4. **Fallback** to heuristic brace-depth parser when `tree.root_node().has_error()` — interpolation (Dart `${}`, JS `${}`, Python f-strings) makes strings invalid GraphQL, but we can still extract resolver names from the structure. The interpolation cleanup pipeline is language-agnostic with language-specific token patterns.

**Heuristic coverage**: Don't rely on `query|mutation|subscription` only. Accept:
- Explicit operations (`query Foo { ... }`, `mutation Bar { ... }`)
- Shorthand query documents starting with `{` (treat as anonymous query)
- Mixed documents with fragments (extract operations, skip fragment-only docs for AppSync linking)
- Skip documents that are only fragment/schema/type definitions

**Confidence scoring**: Weighted evidence model.
- **High (0.95-1.0)**: Valid tree-sitter-graphql parse + operation definition present (+ tagged template boost)
- **Medium (0.7-0.9)**: Valid parse without tag hint, or tag hint without clean parse
- **Low (0.4-0.6)**: Heuristic fallback (interpolation-corrupted strings)
- Tagged templates (`gql\`...\``) boost confidence but don't gate detection

**Parenting strategy**: `Contains` edge to nearest enclosing extracted node (function/method/class/module) by line range comparison. Variable declarations are not consistently extracted today, so variable-parenting is opportunistic. Fallback to file node.

### Embedded DSL Config Registry

Per-language string literal node types (tree-sitter AST node names):

| Language | String Node Types | Tag Hints | Interpolation Pattern |
|----------|------------------|-----------|----------------------|
| TypeScript/JS | `template_string`, `string` | `gql` tagged template | `${...}` |
| Dart | `string_literal` (triple-quoted `'''`/`"""`) | none | `${...}` |
| Python | `string`, `concatenated_string` | none | `{...}` in f-strings |
| Ruby | `heredoc_body`, `string_content` | none | `#{...}` |
| Go | `raw_string_literal`, `interpreted_string_literal` | none | none (no interpolation) |
| Java | `text_block`, `string_literal` | none | none |
| Rust | `raw_string_literal`, `string_literal` | none | none |
| PHP | `heredoc_body`, `string`, `encapsed_string` | none | `{$...}` |
| C# | `verbatim_string_literal`, `raw_string_literal_content` | none | none |
| Swift | `multi_line_string_literal`, `line_string_literal` | none | `\(...)` |

### Files to modify

**`codegraph/crates/codegraph-extraction/src/embedded_dsl.rs`** (new file)
- Define `EmbeddedDslConfig` struct:
  ```rust
  pub struct EmbeddedDslConfig {
      pub string_node_types: Vec<&'static str>,
      pub tag_hints: Vec<&'static str>,         // e.g. ["gql"] for TS/JS
      pub interpolation_pattern: Option<&'static str>, // regex for cleanup
      pub min_string_length: usize,             // default 20
      pub warn_threshold: usize,               // default 100, log warning if exceeded
  }
  ```
- `get_embedded_dsl_config(lang: Language) -> Option<EmbeddedDslConfig>` — returns config for all languages that can contain embedded GraphQL (skip C, C++, Bash, HCL, GraphQL itself)
- `is_graphql_candidate(text: &str) -> bool` — cheap lexical check: contains `{`, starts with keyword or `{`
- `strip_interpolation(text: &str, pattern: &str) -> String` — replace interpolation tokens with placeholder identifiers to produce parseable GraphQL
- `detect_confidence(parsed_ok: bool, has_operation: bool, has_tag_hint: bool, has_interpolation: bool) -> f64` — weighted confidence scoring

**`codegraph/crates/codegraph-extraction/src/tree_sitter_extractor.rs`**
- Add `#[cfg(feature = "lang-graphql")] gql_candidates: Vec<CandidateString>` field to `ExtractionContext`
- Add `collect_string_candidate()` method — called inside `walk_ast()` for each node. Checks if node type matches `EmbeddedDslConfig.string_node_types`, strips quotes, applies cheap `is_graphql_candidate()` prefilter, pushes to `ctx.gql_candidates` with `parent_id` as enclosing node
- After `walk_ast()` scope ends (line 92), add deferred parsing block:
  ```rust
  #[cfg(feature = "lang-graphql")]
  if !gql_candidates.is_empty() {
      run_graphql_pass(gql_candidates, &mut self.parser, file_path, lang, &file_id, &mut result);
  }
  ```
- Add `run_graphql_pass(candidates, parser, file_path, lang, file_id, result)` — for each candidate:
  1. Try tree-sitter-graphql parse → extract operations and top-level selection fields
  2. On parse failure (`has_error()`): strip interpolation, retry parse. If still fails, fall back to `parse_graphql_heuristic()`
  3. Create `GraphQLOperation` node with metadata: `{"operation_type", "operation_name", "resolver_names": [...], "detection_confidence", "parse_mode": "treesitter"|"heuristic", "has_interpolation"}`
  4. Add `Contains` edge from `candidate.enclosing_node_id`
- Add `parse_graphql_with_treesitter(text: &str, parser: &mut TreeSitterParser) -> Option<GraphqlParseResult>` — use GraphQL parser to extract operations and top-level selection fields
- Add `parse_graphql_heuristic(text: &str) -> Option<GraphqlParseResult>` — fallback brace-depth parser:
  - Detect operation type from leading keyword (or assume `query` for shorthand `{...}`)
  - Extract operation name (text between keyword and first `{`)
  - Extract top-level field names at brace depth 1 (these are the resolver names)
  - Handle aliases (`alias: actual_name` — use actual_name)

**`codegraph/crates/codegraph-extraction/src/lib.rs`**
- Add `mod embedded_dsl;`

### Test
```
cargo test -p codegraph-extraction
```
- Test GraphQL detection in TypeScript tagged template (`gql\`query Foo { bar }\``)
- Test GraphQL detection in Dart triple-quoted string
- Test GraphQL detection in Python triple-quoted string
- Test confidence scoring: tagged TS > untagged Dart > heuristic fallback
- Test interpolation cleanup: `${variable}` replaced with placeholder, parse succeeds
- Test shorthand query: `{ users { id name } }` detected as anonymous query
- Test alias handling (`entitlements: guests_searchEntitlementInstances`)
- Test prefilter rejects short strings, strings without `{`, non-GraphQL content
- Test warning log: file with >100 candidates triggers warning but all are processed
- Integration: extract files in 3+ languages with embedded GraphQL, verify `GraphQLOperation` nodes created with correct metadata

---

## Phase 4: Cross-Language Resolution

**Goal**: After extraction + rules-based identification (Phase 2), match `GraphQLOperation` resolver names to backend nodes tagged by resolver rules. Create typed edges.

**Architecture decision**: Use **direct metadata-query resolution**, not the existing `unresolved_refs` pipeline. The unresolved_refs pipeline is name-based and designed for import/call resolution. Resolver linking needs confidence scores, match metadata, and fuzzy matching — a separate resolution pass is cleaner and more maintainable.

**Key change from hardcoded**: Resolution no longer assumes AppSync. It reads `resolver_type` and `resolver_name` from `node.metadata` (populated by Phase 2 rules engine). The `edge_kind` to create comes from the matched rule's `edge_kind` field, stored in the backend node's metadata.

### Files to modify

**`codegraph/crates/codegraph-resolution/src/resolver.rs`**
- Add `ResolverLinkingStats` struct (total_checked, resolved, fuzzy_resolved, unresolved)
- Add `resolve_resolver_links(&mut self) -> Result<ResolverLinkingStats>`:
  1. Build backend registry: query all nodes with `metadata` containing `"resolver_name"` -> map `resolver_name` -> `(node_id, resolver_type, edge_kind)`
  2. Get all `NodeKind::GraphQLOperation` nodes
  3. For each GraphQL node, iterate its `resolver_names` from metadata
  4. **Exact match** (confidence 1.0, or 0.95 for subscriptions): look up resolver name in registry
  5. **Fuzzy match** (opt-in, confidence 0.7+): use existing `NameMatcher::find_all_matches()` from `matcher.rs`
  6. Create edge with the appropriate `EdgeKind` based on `appsync_type`:
     - `"query"` -> `EdgeKind::AppSyncQueryCall`
     - `"mutation"` -> `EdgeKind::AppSyncMutationCall`
     - `"subscription"` -> `EdgeKind::AppSyncSubscription`
  7. Store confidence and match details in edge metadata: `{"resolver_name", "confidence", "match_type"}`
  8. When multiple candidates exist for same name, lower confidence

**`codegraph/crates/codegraph-core/src/codegraph.rs`**
- **Full index path** (`index_all`, ~line 239-246): After `resolve_all()` call, add `resolver.resolve_appsync_links()` call. Log resolution stats.
- **Sync path** (`sync_with_options`, ~line 351-413): After incremental extraction completes, run a scoped AppSync resolution pass. This is critical — without it, `codegraph sync` from child hooks would extract updated nodes but never create/update AppSync edges, making the hooks ineffective.
  - Scope: only re-resolve AppSync links involving changed files (query `GraphQLOperation` nodes and AppSync-decorated nodes in changed file set)
  - Clean up stale edges: when a TS file with `@AppSyncQuery` is re-extracted and the decorator changes or is removed, delete orphaned `AppSyncQueryCall` edges from the previous indexing before creating new ones
  - Use `DELETE FROM edges WHERE source_id IN (changed_graphql_nodes) AND kind IN ('AppSyncQueryCall', ...)` before re-resolving

### Test
```
cargo test -p codegraph-resolution
```
- Integration: in-memory DB with TS node (metadata: appsync query) + Dart GraphQLOperation node -> verify `AppSyncQueryCall` edge created with confidence 1.0
- Test subscription derivation match -> `AppSyncSubscription` edge with confidence 0.95
- Test fuzzy match for slight typo
- Test no match when no backend resolver exists

---

## Phase 5: Graph Traversal Updates

**Goal**: Make AppSync edges visible in callers/callees, call graphs, and impact analysis.

### Files to modify

**`codegraph/crates/codegraph-graph/src/traversal.rs`**
- `get_callers()` (line 182): add `AppSyncQueryCall`, `AppSyncMutationCall`, `AppSyncSubscription` to edge kind filter alongside `Calls`
- `get_callees()` (line 200): same additions
- Impact analysis at lines 517-518, 527-528: add the three AppSync edge kinds alongside `EdgeKind::Calls`

**`codegraph/crates/codegraph-graph/src/queries.rs`**
- `collect_call_graph()` (line 144): add the three AppSync edge kinds to filter
- `get_impact_radius()` (line 175): add the three AppSync edge kinds to filter

**`codegraph/crates/codegraph-context/src/builder.rs`**
- `build_edge_kinds()` (line 334): add AppSync edge kinds alongside `EdgeKind::Calls`. Without this, context building (used by MCP prompts and IDE integration) would silently exclude cross-language relationships.

**`codegraph/crates/codegraph-sync/src/impact.rs`**
- Line 71: hardcoded SQL `e.kind IN ('calls','extends','implements','references')` — **refactor to parameterized query** using `EdgeKind::call_like()` + relationship kinds. Current hardcoded kind SQL will drift as edge kinds expand. Generate the SQL `IN (...)` clause from the enum variants dynamically.

**`codegraph/crates/codegraph-graph/src/traversal.rs`**
- `count_incoming_calls()` (line 488): currently uses raw SQL `kind = 'calls'` — migrate to use `EdgeKind::call_like()` kinds so AppSync callers are counted

**`codegraph/crates/codegraph-graph/src/queries.rs`**
- `find_dead_code()` (~line 290): currently only checks `Calls` edges — add AppSync call-like edges to avoid false "unused" on backend resolvers that are invoked from frontend GraphQL operations

**Required**: Add a helper `EdgeKind::call_like() -> &'static [EdgeKind]` that returns `&[Calls, AppSyncQueryCall, AppSyncMutationCall, AppSyncSubscription]`. Use it in all the above call sites plus `traversal.rs` `get_callers`/`get_callees`, `queries.rs` `collect_call_graph`/`get_impact_radius`, and `impact.rs`. This centralizes the list, avoids N+1 code paths when future API edge kinds are added, and eliminates the class of bugs where a new call-type kind is added to some filters but missed in others.

### Test
```
cargo test -p codegraph-graph
cargo test -p codegraph-sync
```
- Test get_callers returns cross-language callers via AppSync edges
- Test impact analysis traverses AppSync edges
- Test count_incoming_calls includes AppSync call-like edges
- Test find_dead_code does not flag resolvers with AppSync callers as dead

---

## Phase 6: Child Repo Git Hooks

**Goal**: When running `codegraph hooks install` at the parent level (non-git), detect child git repos and install hooks that call `codegraph sync <parent_path>`.

### Files to modify

**`codegraph/crates/codegraph-sync/src/git_hooks.rs`**
- Add `child_hook_script(hook_name, parent_path)` — generates hook that:
  1. Calls `codegraph sync "<parent_path>" --hook <hook_name> --child-repo "$(pwd)" &`
  2. **Diff logic centralized in Rust** (not shell): the CLI `sync` command detects `--child-repo`, runs the appropriate `git diff` variant based on hook type within the Rust code, and scopes extraction to changed files. This is cleaner than shell-side diff because: (a) hook scripts stay simple/portable, (b) diff strategy can evolve without re-installing hooks, (c) CLI already has `--hook` arg infrastructure.
  - This avoids triggering a full hash scan of the entire parent tree on every child commit. Without scoped file lists, every child commit would re-scan all sibling repos — O(total files) instead of O(changed files).
- Add `install_child_hooks(parent_path, force) -> Result<Vec<String>>`:
  - Scan immediate children of `parent_path` for directories containing `.git`
  - For each child, create `GitHooksManager` and install child-specific hooks
  - Return list of child repo names where hooks were installed
- Add corresponding `uninstall_child_hooks(parent_path)`

**`codegraph/crates/codegraph-cli/src/commands.rs`**
- In `hooks_install` (~line 382): if parent path has no `.git` but has child dirs with `.git`, call `install_child_hooks()`
- Print which child repos got hooks installed
- In `hooks_status` (~line 395): add child-aware logic — when at a non-git parent, scan child repos and report hook status for each child. Without this, `codegraph hooks status` at the parent level would report "no hooks" even when child hooks are installed, confusing users.
- In `hooks_uninstall`: call `uninstall_child_hooks()` when at non-git parent

### Test
```
cargo test -p codegraph-sync
```
- Test: temp dir with two child `.git` dirs, verify hook files created with correct parent path
- Test: hook script includes `--files` argument with git diff output
- Test: uninstall removes hooks from children
- Test: `hooks_status` at parent level reports child hook status

---

## Phase 7: MCP Tool

**Goal**: Expose cross-language links through a dedicated MCP tool.

### Files to modify

**`codegraph/crates/codegraph-mcp/src/tools.rs`**
- Add `codegraph_appsync_links` tool:
  - Input: optional `resolver_name`, optional `direction` (frontend_to_backend / backend_to_frontend / both)
  - Queries `AppSyncQueryCall`, `AppSyncMutationCall`, `AppSyncSubscription` edges
  - Returns matches with confidence, file paths, operation types
- Existing `codegraph_callers` and `codegraph_callees` automatically include AppSync edges after Phase 5

### Test
```
cargo test -p codegraph-mcp
```

---

## Phase 8: V2 Decorator Parsing — Contract/Schema Analysis

**Goal**: Extend AppSync decorator parsing to extract contract-level metadata for type matching, auth analysis, and publish permissions.

### Decorators

| Decorator | What we extract | Metadata produced |
|-----------|----------------|-------------------|
| `@SubscriptionModelField()` | Payload field schema (field name, type) | `{"subscription_fields": [{"name": "...", "type": "..."}]}` — enables future type matching between TS subscription payloads and Dart subscriber expectations |
| `@EntityAppSyncType()` | GraphQL type name exposed via AppSync | `{"appsync_graphql_type": "..."}` — enables mapping between entity classes and their GraphQL type representations |
| `@SubscriptionModelAuthGuard()` | Auth configuration (guard type, roles) | `{"auth_guard": {"type": "...", "roles": [...]}}` — enables security analysis of subscription access patterns |
| `@IAMPolicyAccessPublishSubscriptionModel()` | Lambda publish permission config | `{"publish_policy": {"lambda_name": "...", "subscription_model": "..."}}` — enables mapping which Lambdas can publish to which subscription topics |

### Files to modify

**`codegraph/crates/codegraph-extraction/src/tree_sitter_extractor.rs`**
- Extend `parse_appsync_metadata()` to recognize additional decorator patterns
- Add `parse_subscription_field(decorator_text: &str) -> Option<Value>` — extract field name and type from `@SubscriptionModelField()` arguments
- Add `parse_entity_appsync_type(decorator_text: &str) -> Option<Value>` — extract GraphQL type name
- Add `parse_auth_guard(decorator_text: &str) -> Option<Value>` — extract guard config
- Add `parse_publish_policy(decorator_text: &str) -> Option<Value>` — extract Lambda publish permissions
- Merge all decorator metadata into a single `node.metadata` JSON object (multiple decorators on same class produce a combined metadata blob)

### Test
```
cargo test -p codegraph-extraction --features lang-typescript
```
- Test each decorator parser individually
- Test combined metadata from multiple decorators on same class
- Integration: extract a TS file with `@SubscriptionModelField` and verify metadata populated

---

## Phase 9: GraphQLOperation Embeddings

**Goal**: Make `GraphQLOperation` nodes discoverable via semantic search by adding them to the embeddable node kinds and defining an embedding text strategy.

### Embedding content strategy

GraphQLOperation nodes are DSL-in-string — different from typical code nodes. The embedding text should include:
1. **Kind + name**: `"graphql_operation SearchEntitlements"` (operation name from metadata)
2. **File path**: `"in guest-vue-front-end/lib/features/entitlements/data/queries.dart"`
3. **Operation type**: `"operation_type: query"` / `"operation_type: mutation"` / `"operation_type: subscription"`
4. **Resolver names**: `"resolves to: guests_searchEntitlementInstances, guests_getEntitlementDetails"` — the backend method names this operation calls
5. **GraphQL text**: the raw GraphQL string (truncated to `max_snippet_lines`) — enables natural language search like "search entitlements" to find the relevant operation
6. **Enclosing context**: parent function/class name from `Contains` edge — `"in: EntitlementsRepository.searchEntitlements"`

### Files to modify

**`codegraph/crates/codegraph-core/src/codegraph.rs`**
- Add `NodeKind::GraphQLOperation` to `EMBEDDABLE_KINDS` constant (~line 886-897)

**`codegraph/crates/codegraph-vectors/src/text_builder.rs`**
- In `build_text()` (~line 89): add a branch for `NodeKind::GraphQLOperation` that builds embedding text from metadata fields (`operation_type`, `operation_name`, `resolver_names`) rather than the default signature/code approach
- The GraphQL text from the node's body/code serves as the "code snippet" component
- Graph context (callers/callees) still applies — includes the `AppSyncQueryCall` etc. edges for cross-language context

### Test
```
cargo test -p codegraph-vectors
cargo test -p codegraph-core
```
- Test embedding text generation for a `GraphQLOperation` node
- Test that semantic search for "entitlements query" returns GraphQLOperation nodes
- Test token budget truncation: GraphQL text should be truncated before resolver names (resolver names are more important for search relevance)

---

## Phase 10: MCP Workspace Status Reporting

**Goal**: Expose multi-git workspace health through MCP tools — child repo hook status, last sync times, per-repo indexing state.

### Files to modify

**`codegraph/crates/codegraph-mcp/src/tools.rs`**
- Add `codegraph_workspace_status` tool:
  - Input: none (uses current project root)
  - Detects if project root is a multi-git workspace (non-git parent with child `.git` dirs)
  - For each child repo, reports:
    - Hook installation status (installed / not installed / outdated)
    - Last sync timestamp (from DB metadata or file mtime)
    - Number of indexed files / nodes
    - Any sync errors or warnings
  - Returns structured JSON with per-repo status
- Add `codegraph_workspace_sync` tool (optional):
  - Input: optional `repo_name` to sync a specific child, or sync all
  - Triggers `codegraph sync` for the specified child repo(s)
  - Returns sync results with timing and stats

**`codegraph/crates/codegraph-sync/src/git_hooks.rs`**
- Add `get_child_hooks_status(parent_path) -> Result<Vec<ChildHookStatus>>`:
  - For each child repo, check if hooks are installed and up-to-date
  - Return status struct with repo name, hook presence, hook version

**`codegraph/crates/codegraph-db/src/queries.rs`**
- Add `get_file_count_by_prefix(conn, prefix) -> Result<usize>` — count files scoped to a child repo path prefix
- Add `get_node_count_by_file_prefix(conn, prefix) -> Result<usize>` — count nodes in a child repo

### Test
```
cargo test -p codegraph-mcp
cargo test -p codegraph-sync
```
- Test workspace status with multi-git directory structure
- Test status correctly reports hook installation state per child
- Test file/node counts scoped to child repo paths

---

## Verification

### End-to-end test plan
1. Create a test directory structure:
   ```
   /tmp/test-workspace/
     backend/  (TS files with @AppSyncQuery decorators)
     frontend/ (Dart files with GraphQL strings)
   ```
2. `codegraph init /tmp/test-workspace`
3. `codegraph index /tmp/test-workspace`
4. Verify nodes exist for both TS classes and Dart GraphQL operations
5. Verify `AppSyncQueryCall` / `AppSyncSubscription` edges link frontend -> backend
6. MCP tool `codegraph_callers` for a backend resolver ID shows Dart GraphQL operations as callers (no CLI `callers` command exists — use MCP tools or write a test)
7. MCP tool `codegraph_appsync_links` returns correct links with confidence scores

### Run full test suite
```
cargo test --workspace
cargo clippy --workspace --all-targets
```

---

## Design Seams for Future Workspace Support

These deliberate design choices make future multi-root/workspace support addable without rewriting:

1. **Explicit `EdgeKind` variants per API family** — `AppSyncQueryCall` etc. are specific and queryable. Future REST/gRPC linking adds its own kinds (e.g. `RestApiCall`, `GrpcCall`). The `EdgeKind::call_like()` helper makes adding new call-type kinds trivial.
2. **`Node.metadata` is a generic JSON field** — any framework-specific data goes here. No AppSync-specific columns.
3. **`NodeKind::GraphQLOperation` is first-class** — not a hack on Function/Method.
4. **`install_child_hooks(parent_path)` accepts explicit parent** — a future `codegraph workspace add` could call this per-repo.
5. **`resolve_appsync_links()` queries by metadata, not by file path** — does not assume directory structure.
6. **Edge metadata includes `confidence`** — future features can filter by threshold.
7. **No `repos` or `cross_repo_edges` table** — all edges go in the unified `edges` table.
8. **Hybrid GraphQL parsing** — tree-sitter-graphql handles standard GraphQL; heuristic fallback handles non-standard patterns. Future GraphQL schema file support can reuse the same parser.
9. **`EmbeddedDslConfig` registry** — language-agnostic DSL detection pipeline. Future embedded SQL/Prisma detection reuses the same two-pass architecture with different secondary parsers.
10. **Tree-sitter AST queries for resolver rules** — `.scm` query files are the standard tree-sitter pattern matching mechanism. Users can add rules for any framework without code changes.

---

## Resolver Rules Scope

### V1 (Phases 1-7) — cross-language edges via AST queries
| Pattern | AST Query Captures | Edge produced |
|---------|-------------------|---------------|
| `@AppSyncQuery({ methodName: "X" })` | `@resolver_name` = `"X"` from decorator arg | `AppSyncQueryCall` |
| `@AppSyncMutation({ methodName: "X" })` | `@resolver_name` = `"X"` from decorator arg | `AppSyncMutationCall` |
| `@SubscriptionModel()` on class `FooSubscriptionModel` | `@class_name` → transforms → `subscribeToFoo` | `AppSyncSubscription` |
| `@Query('getUser')` (NestJS) | `@resolver_name` = `"getUser"` from first arg | `AppSyncQueryCall` |
| `@QueryMapping` (Spring) | `@resolver_name` = method name | `AppSyncQueryCall` |

Default `.scm` query files shipped for AppSync, NestJS, Spring GraphQL. Users add custom `.scm` queries for other frameworks.

### V2 (Phase 8) — contract/schema analysis (additional AST queries)
| Decorator | Value | Priority |
|-----------|-------|----------|
| `@SubscriptionModelField()` | Payload field schema for type matching | High |
| `@EntityAppSyncType()` | GraphQL type exposed via AppSync | High |
| `@SubscriptionModelAuthGuard()` | Auth config for security analysis | Medium |
| `@IAMPolicyAccessPublishSubscriptionModel()` | Lambda publish permissions | Medium |

V2 queries can use the same `.scm` query infrastructure — additional query files that capture richer metadata.

### Deferred — infrastructure metadata
`@Entity`, `@GlobalSecondaryIndex`, `@FlexConnect*`, `@OpenSearchIndex`, `@KMSKey` — useful for infra/security topology but don't produce cross-language edges. Can be captured via additional `.scm` query rules when needed.

---

## Prerequisites & Assumptions

- **Relationship edges (calls, imports, extends)**: CLAUDE.md line 166 notes these are P0 missing — only `contains` edges exist today. However, the extraction pipeline already emits `unresolved_refs` and the resolution pipeline resolves them into edges (`tree_sitter_extractor.rs:411`, `resolver.rs:155`, `codegraph.rs:236`). AppSync linking does **not** depend on these relationship edges and should not be blocked. The CLAUDE.md docs should be validated and updated separately.
- **All languages compiled by default**: `codegraph-extraction/Cargo.toml` default features set to `all-languages` (15 languages including tree-sitter-graphql). No feature flag changes needed for Phase 3.

---

## Not in Scope (Future)

- **pubspec.yaml path dependencies**: In monorepo mode, shared-dart-lib is just another directory — imports resolve naturally. PackageImport edges could be added later.
- **Workspace mode**: Design seams make this addable without rewriting.
- **Schema diffing**: Detecting when backend changes break frontend contracts.
- **Type matching**: Matching request/response types across TypeScript <-> Dart.
- **Git integration**: Tracking which commits changed cross-repo contracts.
- **Embedded SQL/Prisma detection**: `EmbeddedDslConfig` registry is designed to be extensible. `tree-sitter-sql` and `tree-sitter-prisma` crates exist on crates.io. Adding SQL/Prisma detection reuses the same two-pass architecture with different secondary parsers and heuristics.
