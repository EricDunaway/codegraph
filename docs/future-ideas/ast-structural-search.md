# AST Structural Pattern Search

Add a structural code search tool that matches AST patterns rather than text, enabling code deduplication detection, pattern discovery, and standards enforcement.

## Why

Codegraph currently has two search layers:

| Layer | Answers |
|-------|---------|
| **Graph** | Who calls whom? Impact radius? |
| **Semantic/FTS** | Find symbols related to "auth" |

A structural search layer fills the gap: **"Where does this exact code shape occur?"** — matching syntax trees instead of raw text. This avoids false positives from strings/comments, is robust to formatting and identifier renames, and supports placeholder captures for downstream reasoning.

## Use Cases

- **Code deduplication** — find functions with identical AST shape but different variable names
- **Pattern discovery** — "does a utility like this already exist?" before writing new code
- **Standards enforcement** — detect anti-patterns (e.g. `.unwrap()` in non-test Rust, bare `except` in Python, ad-hoc SQL string building)

## Implementation Approaches

### 1. ast-grep-core (Rust crate, in-process)

Patterns look like real code with `$` meta-variables:

```
$VAR.unwrap()
console.log($ARGS)
if ($COND) { $BODY } else { $OTHER }
```

- Easier for LLMs to generate patterns at runtime
- Rich rule combinators: `all/any/not`, `inside/has/follows/precedes`
- Adds a new Rust dependency
- **Version friction**: ast-grep-language uses tree-sitter 0.25.10, codegraph uses 0.26. Would need custom `Language` trait adapters over our existing grammars (skip `ast-grep-language` entirely).

### 2. Tree-sitter queries (in-process, zero new deps)

Patterns use S-expression syntax targeting grammar node types:

```scheme
(call_expression
  function: (field_expression
    field: (field_identifier) @method)
  (#eq? @method "unwrap"))
```

- Zero new dependencies — already in the workspace
- Maximum precision (exact AST node type matching)
- Harder for LLMs to generate correctly (requires grammar node names)

### 3. Precomputed structural fingerprints (DB-side)

Compute signatures at index time, query by lookup instead of re-parsing:

- **Structural hashes** — hash the AST shape after normalizing identifiers/literals. Same hash = structurally identical. O(1) duplicate detection.
- **AST node-type sequences** — ordered child node types per symbol. Similarity via edit distance.
- **Subtree hashes (Merkle-style)** — hash each subtree level. Shared hash = shared structural fragment. Detects partial duplicates.
- **Structural embeddings** — embed structural signatures into vector space, reuse existing ONNX + sqlite-vss pipeline for similarity search.

This approach complements runtime pattern search: fingerprints answer "what's similar to this code?" while ast-grep answers "where does this specific pattern occur?"

## Proposed Tool

```
codegraph_structural_search(
  pattern: string,       // code-like pattern or S-expression
  language?: string,     // restrict to language
  path_glob?: string,    // restrict to file paths
  node_kind?: string,    // restrict to containing node type
  limit?: int,           // max results
  captures?: bool,       // return captured meta-variables
)
```

Returns: file path, line range, matched text, captures, enclosing `node_id` (links back to the graph).

## Outstanding Questions

- **ast-grep-core vs tree-sitter queries vs both?** ast-grep is better UX for AI agents but adds dependency and version management. Tree-sitter queries are free but harder to author. A hybrid (code-like syntax compiled to tree-sitter queries) is highest effort.
- **How to handle the tree-sitter version gap?** ast-grep-language pins 0.25.10; codegraph is on 0.26. Writing custom `Language` trait adapters is the likely path — how much work is that?
- **Should fingerprints be a separate tool or the same tool?** Precomputed fingerprints (dedup detection) serve a different query shape than runtime pattern search. Possibly `codegraph_similar_code` as a separate tool.
- **Performance at scale** — for large repos, should pattern search be scoped to changed files by default? Use the sync layer's change detection to limit scope?
- **Pattern library** — should there be a way to store and reuse named patterns (e.g. project-specific anti-pattern rules)?
- **Priority relative to relationship edges** — the `calls`, `imports`, `extends` edges are still P0. Does this feature depend on those, or is it fully independent?
