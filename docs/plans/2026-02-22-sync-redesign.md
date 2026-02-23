# Sync Redesign — Efficient Change Detection, Extraction, and Embedding

**Date:** 2026-02-22
**Status:** Design
**Crates affected:** `codegraph-sync`, `codegraph-core`, `codegraph-resolution`, `codegraph-db`, `codegraph-graph`, `codegraph-extraction`, `codegraph-cli`, `codegraph-mcp`

## Purpose

Redesign CodeGraph's sync pipeline from trigger to embedding update. Covers git hook triggers, change detection, safe extraction, scoped resolution, and incremental embedding — as one end-to-end system.

## Non-Goals

- LSP enrichment (see `2026-02-04-embedding-enrichment-design.md`)
- Embedding content quality / token budgets (see enrichment design)
- New language support
- MCP tool behavior changes (minor: `are_hooks_installed()` check updated to include `post-rewrite`)

## Motivation

### Stream A: Resolution & Edge Diff Redesign
Retire EdgeSnapshot from the hot path by making resolution scoped and edge changes trackable without full table scans.

- Edge duplicates accumulate from resolver re-runs (no uniqueness constraint) — add dedup as prerequisite for safe scoped resolution
- Global `resolve_all()` during sync processes every unresolved ref, creating edges from unchanged files — scope resolution to changed files only
- `EdgeSnapshot::capture()` reads ALL edges twice per sync, O(E) x 2 — replace with targeted pre-delete impact capture once scoped resolution and dedup are in place

### Stream B: Hook & Lock Operability
Make sync hook-aware with proper concurrency and failure handling.

- Git hooks run `codegraph sync "$PWD" &` with no change context, triggering O(all files) hash scan — pass change context via git diff in Rust
- `IndexLock` held only inside `SyncManager::sync()`, leaving snapshot/resolution/embedding unlocked — widen lock to cover entire critical section
- When sync is running and a hook fires, the event is lost — add `sync.pending` coalescing with drain loop
- Background sync failures are silent — add `sync.log`/`sync.failed` with `codegraph status` surfacing

### Standalone Fixes
- `process_modify` deletes old nodes BEFORE re-extract succeeds — parse first, transact second
- Node IDs include line numbers, so cosmetic edits churn every ID below the change — deferred to future milestone (amplifies cost but doesn't block correctness)

## Design Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Change detection | Git diff against DB checkpoint; fall back to SHA256 scan | Hooks know something changed; Rust does the diffing |
| Diff base | DB checkpoint (`sync.last_head`), not shell-computed refs | Eliminates shell ref-computation bugs (amend, rebase, null refs) |
| Hook scripts | Identical self-contained scripts, pass hook name via `basename "$0"` | No shared helper file; simple and idempotent |
| Lock scope | Single lock across detect-through-embed | Prevents concurrent modification across entire critical section |
| Lock collision | Write `sync.pending`; running sync drains on completion | No lost events without queue complexity |
| Extraction safety | Parse outside transaction, swap inside transaction | Tree-sitter failure causes no data loss |
| Resolution scope | Two-sided scoped (source + target) during sync | Cheaper than global; full `resolve_all()` only on `codegraph index` |
| Edge snapshot | Replaced with pre-delete impact capture | Targeted queries proportional to changed files, not all edges |
| Edge dedup | Prevent exact duplicates; preserve callsite cardinality | Multiple calls at different lines are distinct; same-line duplicates are not |
| Embed candidates | Changed nodes + pre-delete neighbors + resolver affected + siblings | Over-approximate but guaranteed correct |
| Hook chaining | Wrapper pattern: call `.codegraph-orig` first, forward exit code | Preserves user hooks; CodeGraph errors never block git |
| Failure visibility | `sync.log` (append) + `sync.failed` (overwrite) | Background failures surfaced via `codegraph status` |

## End-to-End Sync Pipeline

```
Phase 0: TRIGGER
    Git hook fires (post-commit/post-checkout/post-merge/post-rewrite)
    OR manual `codegraph sync`
    OR programmatic caller with file list

Phase 1: DETECT CHANGES
    Git hook mode: git diff against sync.last_head checkpoint
    External mode: caller-provided Vec<String>
    Fallback mode: SHA256 hash scan (ChangeDetector)
    Output: Vec<FileChange> { path, kind: Added|Modified|Deleted, language }

Phase 2: ACQUIRE LOCK + REVALIDATE
    Acquire IndexLock (held through Phase 7)
    On LockHeld + hook mode: write sync.pending, exit
    Re-hash detected changed files under lock
    If hash differs from detection, update change list

Phase 3: SAFE EXTRACT WITH PRE-DELETE IMPACT CAPTURE
    For each file change:
    a) Parse new content with tree-sitter (OUTSIDE transaction)
    b) Batch query pre-delete impact (neighbors of old nodes)
    c) Per-file SQLite transaction: delete old + insert new
    Output: changed_file_paths, deleted_node_ids, pre_delete_affected_ids

Phase 4: TWO-SIDED SCOPED RESOLUTION
    Source-scoped: refs FROM changed files
    Target-scoped: refs TO symbols in changed files (frequency-capped)
    INSERT OR IGNORE for edges
    Returns: newly_resolved_node_ids (source + target)

Phase 5: COMPUTE EMBED CANDIDATES
    Union: changed nodes ∪ pre-delete affected ∪ resolver affected ∪ siblings
    Minus: deleted node IDs
    Filter: EMBEDDABLE_KINDS only

Phase 6: EMBED + CLEANUP
    Full re-embed check (should_full_reembed)
    Delete stale vectors, generate embeddings
    Lock heartbeat every 2 minutes
    Update sync.last_head checkpoint
    Record embed metadata

Phase 7: RELEASE LOCK + DRAIN PENDING
    Release IndexLock
    If sync.pending exists: rename to sync.processing, re-run from Phase 1
```

---

## Phase 0: Trigger (Git Hooks)

### Hook Scripts

All 4 hooks use an identical script. The hook name is derived from the filename.

```sh
#!/bin/sh
# CodeGraph auto-sync hook
# Installed by: codegraph hooks install

git rev-parse --is-inside-work-tree >/dev/null 2>&1 || exit 0
repo_root="$(git rev-parse --show-toplevel 2>/dev/null)" || exit 0
[ -d "$repo_root/.codegraph" ] || exit 0

hook_name="$(basename "$0")"

# Chain to original hook if it exists
orig_hook="$(dirname "$0")/${hook_name}.codegraph-orig"
orig_exit=0
if [ -x "$orig_hook" ]; then
    "$orig_hook" "$@" || orig_exit=$?
fi

# Trigger sync in background — errors logged, never block git
if command -v codegraph >/dev/null 2>&1; then
    ( codegraph sync "$repo_root" --hook "$hook_name" \
        >> "$repo_root/.codegraph/sync.log" 2>&1 ) &
fi

exit "$orig_exit"
```

**Hooks installed:** `post-commit`, `post-checkout`, `post-merge`, `post-rewrite`

**Why identical scripts work:** All hooks use the DB checkpoint (`sync.last_head..HEAD`) for diffing, not hook-specific arguments. Hook args are forwarded to chained `.codegraph-orig` hooks via `"$@"`.

### Hook Installation

- Find hooks dir via `git rev-parse --git-path hooks/<name>` (supports worktrees, `core.hooksPath`)
- Conflict detection before install:
  1. **Existing CodeGraph hook** → skip (idempotent)
  2. **Existing non-CodeGraph hook** → rename to `<hook>.codegraph-orig`, install wrapper
     - If `.codegraph-orig` already exists (previous failed install?), refuse and warn user to run `codegraph hooks uninstall` first
  3. **Husky/Lefthook detected** (`core.hooksPath` non-default, `.husky/` exists, `.lefthook.yml` exists) → warn, require `--force`
- Ensure `.codegraph/` in `.gitignore` on both `codegraph hooks install` AND `codegraph init`

### Hook Uninstall

1. Remove CodeGraph hook script
2. Restore `.codegraph-orig` to original name if it exists
3. Clean up `sync.pending`, `sync.processing`
4. Preserve `sync.log` and `sync.failed` (audit trail)

---

## Phase 1: Change Detection

Three modes, same output format (`Vec<FileChange>`):

### Git Hook Mode (fast path, `--hook <name>`)

Uses `detect_changes_git()`:

1. Read `sync.last_head` from metadata table
2. Get current HEAD: `git rev-parse HEAD`
3. Committed changes: `git diff --name-status -z --diff-filter=ACDMRT <checkpoint>..HEAD`
   - For renames (R): collect BOTH old path (delete) and new path (add)
   - For type changes (T): treat as modified
4. Working tree changes: `git diff --name-status -z HEAD`
5. Untracked files: `git ls-files --others --exclude-standard -z`
6. Union all three → candidate list
7. Filter to supported languages + exclude patterns
8. Classify against `files` table:
   - Not in `files` table → Added
   - In `files` table, exists on disk → Modified
   - In `files` table, missing from disk → Deleted

**Early exit:** If candidate list is empty after filtering → skip sync, update `sync.last_head` checkpoint, return.

**Fallback triggers:** Missing checkpoint, unreachable ref, git command failure → full scan.

**Non-UTF8 paths:** Treat as errors — log warning and skip the file.

### External File List Mode (programmatic callers)

Caller provides `Vec<String>` directly. Same classification logic. Useful for IDE integrations, filesystem watchers, test harnesses.

### Fallback Mode (manual `codegraph sync`)

`ChangeDetector` SHA256 hash scan. O(all files). Used for non-git repos, CI, first sync after init.

---

## Phase 2: Lock + Revalidate

**Lock scope expansion:**
```
Current:   snapshot(unlocked) → sync(locked) → resolve(unlocked) → embed(unlocked)
Proposed:  LOCK → revalidate → extract → resolve → embed → UNLOCK
```

**Revalidation:** Re-hash only detected changed files under lock. If content changed between detection and lock acquisition, update the change list. O(changed files), not O(all files).

**Lock collision (hook mode):** On `LockHeld`, write `sync.pending` atomically (write to temp file, then rename) instead of failing:
```json
{"hook":"post-commit","timestamp":"2026-02-22T18:30:12Z"}
```

The running sync drains pending on completion via rename-based CAS:
1. Rename `sync.pending` → `sync.processing` (atomic claim)
2. New hook events write fresh `sync.pending` (not lost — atomic write to new file)
3. Process `sync.processing`, delete it
4. Loop back to check for new `sync.pending`

**Lock heartbeat:** `lock.refresh()` every 2 minutes during long phases to prevent 5-minute stale detection.

---

## Phase 3: Safe Extract with Pre-Delete Impact Capture

**Safety fix:** Current `process_modify` deletes old nodes before re-extract. Parse first, transact second:

```rust
// 1. Parse OUTSIDE transaction — failure preserves old data
let content = std::fs::read_to_string(&full_path)?;
let result = self.registry.extract(&content, &change.path, change.language)?;

// 2. Query pre-delete impact BEFORE deletion
let old_nodes = queries.get_nodes_by_file(conn, &change.path)?;
let old_ids: Vec<&str> = old_nodes.iter().map(|n| n.id.0.as_str()).collect();
let affected = batch_pre_delete_impact(conn, &old_ids)?;

// 3. Atomic swap inside transaction
let tx = conn.transaction()?;
queries.delete_nodes_by_file(&tx, &change.path)?;  // CASCADE deletes edges
queries.insert_nodes(&tx, &result.nodes)?;
queries.insert_edges(&tx, &result.edges)?;
// ... insert unresolved refs, update file record ...
tx.commit()?;
```

**Pre-delete impact query** (chunked <= 500 IDs):
```sql
SELECT DISTINCT e.source AS affected_id FROM edges e
WHERE e.target IN (?..?) AND e.kind IN ('calls','extends','implements','references')
UNION
SELECT DISTINCT e.target AS affected_id FROM edges e
WHERE e.source IN (?..?) AND e.kind IN ('calls','extends','implements','references')
```

Plus sibling detection via Contains parent lookup.

---

## Phase 4: Two-Sided Scoped Resolution

**Source-scoped:** Resolve unresolved refs where source node's file is in `changed_files`. These are freshly extracted refs needing resolution.

**Target-scoped:** Find unresolved refs in ANY file whose `reference_name` matches names of symbols added/changed in `changed_files`. These are old refs that couldn't resolve before but now can.

**Frequency cap:** If a name matches > 100 unresolved refs, skip target-scoped for that name. Defer to periodic `codegraph index`.

**Edge insertion:** `INSERT OR IGNORE` (after uniqueness constraint) prevents duplicates.

**Retain resolved refs:** `resolved BOOLEAN DEFAULT 0` on `unresolved_refs`. Set true on resolution instead of deleting. Enables re-resolution when targets change.

**Return value:** `ResolveResult` includes BOTH source and target node IDs of newly created edges.

Global `resolve_all()` runs only on `codegraph index` (full reindex), not on sync.

---

## Phase 5: Embed Candidate Computation

```
embed_candidates = (
    nodes_in_changed_files           // direct changes
    ∪ pre_delete_affected_ids        // neighbors of deleted/modified nodes (Phase 3)
    ∪ newly_resolved_source_ids      // sources of new resolver edges (Phase 4)
    ∪ newly_resolved_target_ids      // targets of new resolver edges (Phase 4)
    ∪ siblings_of_changed_or_deleted // Contains-based siblings
) - truly_deleted_node_ids           // exclude nodes that no longer exist
```

Filter to `EMBEDDABLE_KINDS`: function, method, class, struct, interface, trait, component.

---

## Phase 6: Embed + Cleanup

1. Check `should_full_reembed()` — if triggered (schema/config/model change), do full re-embed instead
2. Delete stale vectors for `deleted_node_ids` (chunked <= 500)
3. Generate embeddings for candidates (`build_embedding_text` + ONNX)
4. `lock.refresh()` every 2 minutes during batch embedding
5. Update `sync.last_head` = current HEAD if in git-hook mode (no-op for non-git fallback)
6. Record embed metadata on success
7. Clear `sync.failed` if present
8. Graceful degradation if ONNX/model unavailable

---

## Phase 7: Release Lock + Drain Pending

1. Release `IndexLock`
2. Check for `sync.pending`
3. If present: rename to `sync.processing` (atomic claim), re-run from Phase 1
4. Delete `sync.processing` after completion
5. Loop until no more pending

---

## CLI Changes

```
codegraph sync [path]                    # Manual sync (full scan)
codegraph sync [path] --hook <name>      # Hook-triggered (git-diff based)
codegraph sync [path] --verify-sync      # Run with EdgeSnapshot verification
```

### Metadata Keys

| Key | Value | Purpose |
|-----|-------|---------|
| `sync.last_head` | SHA-1 hex | Last successfully synced git HEAD |
| `sync.last_timestamp` | ISO 8601 | When last sync completed |

---

## Schema Migrations

### 1. Edge Dedup

```sql
-- Deduplicate existing rows
DELETE FROM edges WHERE rowid NOT IN (
    SELECT MIN(rowid) FROM edges
    GROUP BY source, target, kind, COALESCE(line, -1), COALESCE(col, -1)
);

-- Add unique index
CREATE UNIQUE INDEX IF NOT EXISTS idx_edges_unique
ON edges(source, target, kind, COALESCE(line, -1), COALESCE(col, -1));
```

Switch edge insertion to `INSERT OR IGNORE` in extraction and resolution.

Multiple calls from the same function to the same target at different lines ARE distinct edges (preserves call frequency signal). Same-line duplicates are collapsed.

### 2. Unresolved Refs Retention

```sql
ALTER TABLE unresolved_refs ADD COLUMN resolved INTEGER DEFAULT 0;
CREATE INDEX IF NOT EXISTS idx_unresolved_resolved ON unresolved_refs(resolved, reference_name);
```

### 3. Scoped Resolution Indexes

```sql
CREATE INDEX IF NOT EXISTS idx_unresolved_name_resolved
ON unresolved_refs(reference_name, resolved);
```

---

## Concurrency and Recovery

### Lock Lifecycle

```
Hook fires → try_acquire()
├─ Acquired → run Phases 2-7, release, drain pending
└─ LockHeld → write sync.pending, exit

sync.pending exists when current sync completes:
├─ rename sync.pending → sync.processing (atomic claim)
├─ re-run Phases 1-7
├─ delete sync.processing
└─ check for new sync.pending (loop)
```

### Stale Lock Recovery

- 5-minute mtime-based stale detection (existing)
- Lock heartbeat (`refresh()`) every 2 minutes prevents false stale breaks
- PID written to lock file for debugging

### Shutdown Race

A hook writing `sync.pending` between the final pending check and lock release creates an unprocessed event. Acceptable — the next hook trigger or manual sync catches it.

---

## Observability

### sync.log

- Plain text, stdout+stderr from background sync
- Location: `.codegraph/sync.log`
- Written by hook script redirect

### sync.failed

- Single line, overwritten on each failure
- Content: `<timestamp> hook=<name> exit=<code> error=<message>`
- Cleared on successful sync
- Surfaced by `codegraph status`

---

## Security

- `sync.pending` contents validated: hook name must be allowlisted, no arbitrary args
- `sync.log`/`sync.failed` — shell redirect (`>>`) happens before Rust runs, so Rust-side path validation cannot protect against symlink tricks on `sync.log`. Accepted as local trust boundary (`.codegraph/` is user-owned). Create with restrictive permissions where possible.
- `.codegraph-orig` execution: inherent trust of user's existing hooks; same security model as git itself
- `command -v codegraph` may resolve unexpectedly in compromised PATH — accepted risk (same as any git hook)

---

## Full Reindex Fallback

If `changed_files.len() > 0.3 * total_tracked_files`, fall back to full reindex. At this scale, scoped resolution savings diminish and full `resolve_all()` is more reliable.

**Fix required:** Current `index_all()` does not clear `nodes/edges/files` tables before rebuild — only clears `unresolved_refs`. Must add full table clear.

---

## Known Limitations

### Node ID Instability

`generate_node_id(file_path, kind, name, line)` includes line number. Adding a blank line churns every ID below it, inflating the ripple set.

**Impact:** Correctness preserved. Efficiency reduced for cosmetic edits.

**Future fix:** Stable IDs using `file_path:kind:name:scope` (parent container name). Requires migration.

### Resolver Precision

Resolver deletes unresolved refs by `(from_node_id, reference_name)` only, not by kind or line. Can drop distinct refs incorrectly. The `resolved` boolean mitigates but doesn't fully fix.

### Target-Scoped Resolution on Common Names

Even with frequency cap (> 100 refs), moderately common names add overhead. Threshold may need tuning.

### Git Hook Edge Cases

- **Submodules:** Gitlink entries, not file paths. Each submodule needs its own CodeGraph index.
- **`--name-status -z` parser:** Rename status includes similarity score (`R100`). Parser must handle `R\d+`/`C\d+` prefixes.
- **GUI git clients:** May have restricted PATH. `command -v` check handles gracefully (silent no-op).

---

## EdgeSnapshot Verification Mode

Retained as optional `--verify-sync` debug tool, off hot path:

1. Capture pre-sync EdgeSnapshot (full)
2. Run normal sync (Phases 1-7)
3. Capture post-sync EdgeSnapshot (full)
4. EdgeDiff: compare affected_nodes against Phase 5 candidates
5. Log discrepancies

Validates that the scoped approach produces correct results.

---

## Future Extensions

### Branch-Based Databases

With git hooks tracking branch context via `post-checkout`:

```
.codegraph/
  branches/
    main/codegraph.db
    feature-x/codegraph.db
    bugfix-y/codegraph.db
```

On branch switch: open branch-specific DB. If none exists, copy from base branch and sync.

- Storage: ~1x per active branch (acceptable)
- SQLite WAL: must close connections before switch
- Branch cleanup: prune DBs for merged/deleted branches
- Config: `branch_databases: true/false` in `.codegraph/config.json`

### Filesystem Watch Mode

Long-running `codegraph watch` daemon using FSEvents (macOS) / inotify (Linux). Hooks remain source of truth; FS events optimize latency.

### Pre-commit Staged Preview

Optional `pre-commit` hook pre-computing candidates from `git diff --cached`. Reduces post-commit sync latency. Not default (adds commit latency).

---

## Implementation Order

Streams A and B can be developed in parallel. Each milestone is independently shippable — intermediate states are acceptable (e.g., hooks land before failure visibility; sync uses narrow lock until M3 widens it).

### Milestone 1: Standalone Fixes (Problem #3)
1. **Fix process_modify safety** — parse before delete, per-file transactions (`codegraph-sync`)
2. **Fix index_all() clear** — full table clear before rebuild (`codegraph-core`)

### Milestone 2: Stream A — Resolution & Edge Diff (Problems #5 → #4 → #1)
3. **Schema migrations** — edge dedup unique index, unresolved_refs `resolved` column, new indexes (`codegraph-db`)
4. **Scoped resolution API** — `resolve_for_files(changed_files)` with two-sided scoping, INSERT OR IGNORE (`codegraph-resolution`)
5. **Pre-delete impact capture** — batched neighbor query replacing EdgeSnapshot (`codegraph-sync`, `codegraph-graph`)
6. **Wire new sync flow (fallback mode)** — integrate Phases 2-7 in `CodeGraph::sync()` using ChangeDetector, scoped resolution, pre-delete capture. Uses existing (narrow) lock scope until M3 widens it. (`codegraph-core`)

### Milestone 3: Stream B — Hook & Lock Operability (Problems #2 → #7 → #8 → #9)
7. **Expand lock scope** — hold across detect → embed, time-based refresh (`codegraph-sync`, `codegraph-core`)
8. **`detect_changes_git()`** — git diff parsing in Rust with `--name-status -z` (`codegraph-sync`)
9. **Checkpoint management** — `sync.last_head` read/write in metadata (`codegraph-db`)
10. **Wire hook mode into sync flow** — add Phase 0+1 git-diff path to `CodeGraph::sync()` (`codegraph-core`)
11. **`--hook` CLI flag + pending sync** — hook-triggered sync path with `sync.pending` collision recovery (`codegraph-cli`, `codegraph-sync`)
12. **New hook scripts** — identical template, chaining, safety guards (`codegraph-sync`)
13. **Hook installation** — `git rev-parse --git-path hooks`, conflict detection, gitignore (`codegraph-sync`)
14. **Failure visibility** — `sync.log`, `sync.failed`, status surfacing (`codegraph-cli`)

### Milestone 4: Verification and Polish
16. **Verification mode** — EdgeSnapshot as `--verify-sync` debug tool (`codegraph-core`)
17. **Integration tests** — hook install/uninstall, checkpoint sync, pending recovery, scoped resolution (`codegraph-sync`, `codegraph-core`)

### Future
18. **Stable node IDs** — remove line number from ID hash (Problem #6) (`codegraph-extraction`)
19. **Branch-based databases** — per-branch DB support (`codegraph-sync`, `codegraph-core`)
20. **Filesystem watch mode** — FSEvents/inotify daemon

---

## Performance Expectations

| Scenario | Current | Proposed |
|----------|---------|----------|
| 1 file changed, 10K node graph | ~200ms (2x full edge scan + hash scan) | ~20ms (scoped queries) |
| 10 files changed, 50K edges | ~500ms | ~80ms |
| Branch switch, 100 files diverged | Full re-sync (~5s+) | ~200ms (git diff + scoped sync) |
| Branch switch (with branch DBs) | Full re-sync (~5s+) | Instant (DB swap) |
| No changes detected | ~100ms (edge scan still runs) | ~5ms (skip everything) |
| Hook fires during active sync | Event lost | Coalesced via sync.pending |

*Estimates assume CoreML-accelerated embedding and warm SQLite page cache.*

---

## Affected Files

| File | Changes |
|------|---------|
| `crates/codegraph-sync/src/sync.rs` | Per-file transactions, pre-delete capture, pending mechanism, lock scope |
| `crates/codegraph-sync/src/change_detector.rs` | New `detect_changes_git()` method |
| `crates/codegraph-sync/src/git_hooks.rs` | New scripts, git-path hooks, conflict detection, post-rewrite |
| `crates/codegraph-sync/src/lock.rs` | Pending-write on LockHeld, heartbeat integration |
| `crates/codegraph-sync/src/edge_diff.rs` | Move to optional verification mode |
| `crates/codegraph-core/src/codegraph.rs` | New sync flow (Phases 1-7), lock scope, checkpoint |
| `crates/codegraph-resolution/src/resolver.rs` | New `resolve_for_files()`, retain resolved refs |
| `crates/codegraph-db/src/schema.rs` | Edge unique index, unresolved_refs resolved column |
| `crates/codegraph-db/src/queries.rs` | INSERT OR IGNORE for edges, metadata helpers |
| `crates/codegraph-db/src/migrations.rs` | Schema migration for edge unique index + unresolved_refs column |
| `crates/codegraph-cli/src/main.rs` | `--hook`, `--verify-sync` flags |
| `crates/codegraph-cli/src/commands.rs` | sync.failed surfacing in status |
| `crates/codegraph-mcp/src/git.rs` | Add post-rewrite to `are_hooks_installed()` |

---

## Appendix: Codex Review Summary

Developed through 6 iterative Claude-Codex rounds. Key corrections from Codex:

1. Relationship extraction IS partially implemented (unresolved refs emitted for calls/imports/extends/implements). The gap is resolver quality, not extraction.
2. `resolve_all()` is global and creates edges from unchanged files — pre-delete neighbor query alone is insufficient without scoped resolution.
3. `get_embedding_neighbors()` is unused in production — sync uses `EdgeDiff` directly.
4. `capture_for_files()` exists but is unused — full `capture()` used instead.
5. Resolved refs are deleted from `unresolved_refs`, preventing re-resolution when targets change.
6. Node IDs include line numbers, causing massive ID churn on cosmetic edits.
7. `process_modify` deletes before successful re-extract — data loss risk.
8. No edge uniqueness constraint — duplicates accumulate from resolver.
9. Lock scope race is wider than just snapshot — resolution and embedding also run unlocked.
