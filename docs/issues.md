# CodeGraph Issues

Bugs and issues where our capabilities don't match our own spec/schema.

## Extraction Issues

### `code_snippet` not populated (P0)

**Schema:** `nodes.code_snippet TEXT` exists
**Actual:** NULL for all nodes
**Impact:** `codegraph_node` can't show actual code, user must read file manually
**Location:** `crates/codegraph-extraction/`

```sql
-- Verify issue:
SELECT COUNT(*) FROM nodes WHERE code_snippet IS NOT NULL;
-- Returns: 0
```

### `signature` not populated (P1)

**Schema:** `nodes.signature TEXT` exists
**Actual:** NULL for most nodes
**Impact:** Can't see function signatures without reading source
**Location:** `crates/codegraph-extraction/`

### `docstring` not populated (P1)

**Schema:** `nodes.docstring TEXT` exists
**Actual:** NULL for most nodes
**Impact:** Can't see documentation without reading source
**Location:** `crates/codegraph-extraction/`

### Only `contains` edges created - NO relationship edges (P0 CRITICAL)

**Schema:** `edges` table with kinds: `calls`, `imports`, `exports`, `extends`, `implements`, `references`, `type_of`, `returns`, `instantiates`, `overrides`, `decorates`
**Actual:** Only `contains` edges exist (parent→child structural relationships)
**Impact:** Call graph is completely broken - `codegraph_callers`, `codegraph_callees`, `codegraph_impact` return empty results
**Location:** `crates/codegraph-extraction/`

```sql
-- Verify issue:
SELECT kind, COUNT(*) FROM edges GROUP BY kind;
-- Returns: contains|2064 (ONLY contains edges!)
-- Expected: calls, imports, extends, etc.
```

**Root cause:** Extraction creates structural `contains` edges (file→function, class→method) but does NOT analyze function bodies to create `calls` edges or import statements to create `imports` edges.

**Fix needed:** After extracting nodes, must:
1. Parse function bodies to find function calls → create `calls` edges
2. Parse import statements → create `imports` edges
3. Parse class declarations for extends/implements → create those edges

**MCP tools affected:**
- `codegraph_callers` → returns "No callers found" (no `calls` edges exist)
- `codegraph_callees` → returns "No callees found" (no `calls` edges exist)
- `codegraph_impact` → returns "0 nodes affected" (no relationship edges to traverse)

## MCP Tool Issues

### `codegraph_node` doesn't output code_snippet (P0)

**Expected:** Tool should show actual source code
**Actual:** Only shows file:line pointer, signature (if exists), docstring (if exists)
**Impact:** Tool doesn't fulfill its purpose of showing code
**Location:** `crates/codegraph-mcp/src/tools.rs:442` (`tool_node` function)

**Fix:** Add code_snippet to output, or read from file using line numbers if snippet not stored.

---

*Last updated: 2026-02-05*
