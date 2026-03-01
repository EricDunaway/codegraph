# CodeGraph Issues

Bugs and issues where our capabilities don't match our own spec/schema.

## Extraction Issues

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

---

*Last updated: 2026-02-28*
