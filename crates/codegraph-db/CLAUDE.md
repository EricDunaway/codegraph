# codegraph-db

SQLite database layer for CodeGraph. Wraps `rusqlite` with schema management, migrations, prepared queries, an in-memory node cache, and enrichment dependency tracking.

## Source Layout

```
src/
  lib.rs              # Public re-exports
  connection.rs       # DatabaseConnection (open, pragmas, schema, migrations)
  schema.rs           # SCHEMA constant (v1 DDL)
  migrations.rs       # run_migrations, migrate_to_v2, migrate_to_v3
  queries.rs          # QueryBuilder (all CRUD, search, stats, metadata)
  enrichment_deps.rs  # Free functions for enrichment_deps table
  error.rs            # DbError enum
```

## Database Schema (v3)

### nodes
Code symbols (functions, classes, methods, etc.).

| Column | Type | Notes |
|--------|------|-------|
| id | TEXT PK | Deterministic hash |
| kind | TEXT | NodeKind string |
| name | TEXT | Symbol name |
| qualified_name | TEXT | Fully qualified name |
| file_path | TEXT | Source file |
| language | TEXT | Language string |
| start_line, end_line | INTEGER | Line span |
| start_column, end_column | INTEGER | Column span |
| docstring | TEXT | Nullable |
| signature | TEXT | Nullable |
| visibility | TEXT | Nullable (public/private/etc.) |
| is_exported | INTEGER | Bool as 0/1 |
| is_async | INTEGER | Bool as 0/1 |
| is_static | INTEGER | Bool as 0/1 |
| is_abstract | INTEGER | Bool as 0/1 |
| decorators | TEXT | JSON array, nullable |
| type_parameters | TEXT | JSON array, nullable |
| updated_at | INTEGER | Unix timestamp |
| inferred_type | TEXT | v2: LSP-derived type |
| resolved_import_path | TEXT | v2: resolved import |
| code_snippet | TEXT | v2: source code text |
| thrown_errors | TEXT | v2: JSON array, default '[]' |
| test_names | TEXT | v2: JSON array, default '[]' |
| package_name | TEXT | v2: nullable |

**Indexes:** file_path, kind, name, qualified_name, language

### edges
Relationships between nodes.

| Column | Type | Notes |
|--------|------|-------|
| id | INTEGER PK | AUTOINCREMENT |
| source | TEXT FK | -> nodes(id) ON DELETE CASCADE |
| target | TEXT FK | -> nodes(id) ON DELETE CASCADE |
| kind | TEXT | EdgeKind string |
| metadata | TEXT | JSON object, nullable |
| line | INTEGER | Nullable |
| col | INTEGER | Nullable |

**Indexes:** source, target, kind, (source,kind), (target,kind)
**Unique index (v3):** `(source, target, kind, COALESCE(line,-1), COALESCE(col,-1))` -- deduplication. Use `INSERT OR IGNORE` for edges.

### files
Tracked source files.

| Column | Type | Notes |
|--------|------|-------|
| path | TEXT PK | Relative file path |
| content_hash | TEXT | For change detection |
| language | TEXT | |
| size | INTEGER | Bytes |
| modified_at | INTEGER | Unix timestamp |
| indexed_at | INTEGER | Unix timestamp |
| node_count | INTEGER | Default 0 |
| errors | TEXT | JSON array, nullable |

**Indexes:** language, modified_at

### unresolved_refs
References pending resolution (two-pass extraction).

| Column | Type | Notes |
|--------|------|-------|
| id | INTEGER PK | AUTOINCREMENT |
| from_node_id | TEXT FK | -> nodes(id) ON DELETE CASCADE |
| reference_name | TEXT | Symbol name being referenced |
| reference_kind | TEXT | EdgeKind string |
| line | INTEGER | |
| col | INTEGER | |
| candidates | TEXT | JSON array, nullable |
| resolved | INTEGER | v3: 0=pending, 1=resolved (soft delete) |

**Indexes:** from_node_id, reference_name, (reference_name, resolved)

### enrichment_deps (v2)
Tracks which files an LSP-enriched node depends on.

| Column | Type | Notes |
|--------|------|-------|
| node_id | TEXT | Composite PK, FK -> nodes(id) ON DELETE CASCADE |
| depends_on_file | TEXT | Composite PK |

**Indexes:** depends_on_file

### metadata (v2)
Key-value store for configuration (embedding config hash, model hash, etc.).

| Column | Type |
|--------|------|
| key | TEXT PK |
| value | TEXT |

### schema_version
Migration tracking.

| Column | Type |
|--------|------|
| version | INTEGER PK |
| applied_at | INTEGER |
| description | TEXT |

### nodes_fts (FTS5 virtual table)
Full-text search over nodes. Content-sync table backed by `nodes`.

**Indexed columns:** id, name, qualified_name, docstring

Kept in sync via triggers: `nodes_fts_insert`, `nodes_fts_delete`, `nodes_fts_update`.

### vectors (not in this crate)
The `vectors` table is created and managed by `codegraph-vectors`, not here.

## Key Public API

### DatabaseConnection (`connection.rs`)

- `open(path)` -- Opens/creates DB, sets WAL mode + pragmas, applies schema + migrations
- `open_in_memory()` -- For tests
- `conn()` -- Raw `&Connection` access
- `queries()` -- `&QueryBuilder` access
- `transaction()` -- Begin a `rusqlite::Transaction`

**Pragmas set on open:** `journal_mode=WAL`, `foreign_keys=ON`, `synchronous=NORMAL`, `cache_size=-64000` (64MB), `temp_store=MEMORY`

### QueryBuilder (`queries.rs`)

**Node ops:**
- `insert_node`, `insert_nodes`, `update_node` -- Upsert semantics (`INSERT OR REPLACE`)
- `delete_node`, `delete_nodes_by_file`
- `get_node_by_id` -- LRU-cached (1000 entries)
- `get_nodes_by_file`, `get_nodes_by_kind`, `get_nodes_in_file`

**Edge ops:**
- `insert_edge`, `insert_edges` -- Uses `INSERT OR IGNORE` (unique index dedup)
- `delete_edges_by_source`
- `get_outgoing_edges`, `get_incoming_edges` -- Optional kind filter

**Search:**
- `search_nodes(query, kinds, languages, limit, offset)` -- FTS5 first, LIKE fallback
- `search_symbols(query)` -- Simplified wrapper, returns `Vec<Node>`

**File tracking:**
- `upsert_file`, `delete_file` (also deletes file's nodes), `get_file_by_path`, `get_all_files`

**Unresolved refs:**
- `insert_unresolved_ref`, `insert_unresolved_reference` (simple version)
- `get_all_unresolved_refs` -- Only `resolved=0`
- `get_unresolved_by_name`, `get_unresolved_references` (tuple form)
- `get_unresolved_refs_by_files` -- Source-scoped, `resolved=0` only
- `get_symbol_names_in_files` -- Target-scoped resolution helper
- `get_unresolved_refs_by_names_capped(names, cap)` -- Skips names with > cap refs
- `mark_unresolved_ref_resolved` -- Soft delete (sets `resolved=1`)
- `delete_unresolved_reference`, `clear_unresolved_refs`

**Metadata:**
- `set_metadata`, `get_metadata`, `delete_metadata`, `get_all_metadata`

**Stats:**
- `get_stats()` -- Returns `GraphStats` with counts broken down by kind/language

**Maintenance:**
- `clear()` -- Deletes all data from core tables
- `clear_all_graph_data()` -- Same, used by `index_all()` for clean rebuilds
- `clear_cache()` -- Flushes the in-memory LRU node cache

### Enrichment deps (`enrichment_deps.rs`, free functions)

- `insert_enrichment_dep`, `insert_enrichment_deps_batch`
- `get_deps_for_node`, `get_nodes_depending_on_file`, `get_nodes_depending_on_files`
- `clear_deps_for_node`, `clear_deps_for_nodes`, `clear_deps_for_file`
- `count_deps_for_node`

Batch queries auto-chunk at 500 items to prevent oversized SQL.

### Migrations (`migrations.rs`)

- `run_migrations(conn)` -- Runs all pending migrations (called automatically on `open`)
- `get_schema_version(conn)` -- Returns current version from `schema_version` table
- `migrate_to_v2` -- Adds enrichment columns, `enrichment_deps` table, `metadata` table
- `migrate_to_v3` -- Deduplicates existing edges, adds unique index, adds `resolved` column to `unresolved_refs`

All migrations are idempotent (check version before running).

## Node Cache Pattern

`QueryBuilder` maintains an in-memory LRU cache (max 1000 nodes) for `get_node_by_id`. Cache is invalidated on `update_node`, `delete_node`, `delete_nodes_by_file`, `clear`, and `clear_all_graph_data`. The cache is **not** shared across connections.

## Gotchas

1. **CASCADE deletes.** Deleting a node cascades to `edges`, `unresolved_refs`, and `enrichment_deps` via foreign keys. Capture `EdgeSnapshot` BEFORE deleting nodes if you need pre-delete edge state.

2. **FTS5 content-sync triggers.** The `nodes_fts` table is kept in sync via SQLite triggers on insert/update/delete. Do not write to `nodes_fts` directly. If the FTS index gets corrupted, rebuild with `INSERT INTO nodes_fts(nodes_fts) VALUES('rebuild')`.

3. **FTS5 query syntax.** `search_nodes_fts` sanitizes input (strips `'`, `"`, `*`, `(`, `)`) and appends `*` for prefix matching. Special characters in search terms can cause FTS parse errors; the code catches these and returns empty results.

4. **Edge dedup via unique index (v3).** Edges are deduplicated on `(source, target, kind, COALESCE(line,-1), COALESCE(col,-1))`. Always use `INSERT OR IGNORE` for edge inserts (already done in `insert_edge`).

5. **Unresolved refs are soft-deleted (v3).** `mark_unresolved_ref_resolved` sets `resolved=1` instead of deleting. All query methods filter on `resolved=0` by default. `delete_unresolved_reference` and `clear_unresolved_refs` do hard deletes.

6. **QueryBuilder needs `&mut self` for node mutations.** Methods that modify nodes (`update_node`, `delete_node`, `delete_nodes_by_file`, `clear`, `clear_all_graph_data`) take `&mut self` to invalidate the cache. Read-only methods take `&self` except `get_node_by_id` (also `&mut self` for cache population).

7. **`clear_all_graph_data` vs `clear`.** Both delete the same tables (nodes, edges, files, unresolved_refs). They are functionally identical. Neither touches `metadata` or `schema_version`.

8. **Manual transactions in `enrichment_deps.rs`.** `insert_enrichment_deps_batch` manages its own `BEGIN/COMMIT/ROLLBACK` using raw SQL, not `conn.transaction()`. Do not nest it inside another transaction.

9. **`db_size_bytes` in `GraphStats`.** Always returned as 0 from `get_stats()`. The caller is expected to fill it in from the filesystem.
