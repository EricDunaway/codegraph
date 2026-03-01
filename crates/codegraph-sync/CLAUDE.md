# codegraph-sync

Incremental sync engine for CodeGraph. Detects file changes (filesystem walk or git diff), acquires an exclusive lock, re-extracts changed files, computes which nodes need re-enrichment (with transitive cascade), and drains pending hook events. All state lives in SQLite (`metadata` table) and `.codegraph/` marker files.

## 7-Phase Sync Pipeline

```
Trigger --> Detect --> Lock --> Extract --> Resolve --> Embed --> Release/Drain
```

| Phase | What happens | Key code |
|-------|-------------|----------|
| **1. Trigger** | Git hook fires (`post-commit`, `post-checkout`, `post-merge`, `post-rewrite`) or user runs `codegraph sync`. Hook script calls `codegraph sync --hook <name>` in background. | `git_hooks.rs` hook script, `pending.rs` |
| **2. Detect** | Identify changed files. Two strategies: `ChangeDetector` does a full filesystem walk + SHA256 hash comparison against `files` table. `GitDiffDetector` uses `git diff --name-status -z` between checkpoint and HEAD (faster for hook-triggered syncs). Git detector merges three sources: committed changes, working tree changes, and untracked files. | `change_detector.rs`, `git_diff.rs`, `checkpoint.rs` |
| **3. Lock** | Acquire `index.lock` (exclusive, PID-verified, mtime-heartbeat). If lock is held and caller is a hook, write `sync.pending` instead of blocking (`try_acquire_or_pending`). | `lock.rs`, `pending.rs` |
| **4. Extract** | `SyncManager::process_changes()` iterates `FileChange` list. For modified files, new content is parsed BEFORE old nodes are deleted (parse-first safety). For deletes, `ImpactCapture` grabs neighbor/sibling IDs while edges still exist. | `sync.rs` `process_add`/`process_modify`/`process_delete` |
| **5. Resolve** | `SelectiveScope` determines which nodes/files need re-enrichment. Supports transitive cascade via `enrichment_deps` table (`cascade_depth` controls how many levels deep). | `selective.rs`, `edge_diff.rs` |
| **6. Embed** | `check_reembed_triggers()` decides if a full re-embed is needed (schema migration, config change, model change, force flag) or if incremental is sufficient. Caller (codegraph-core) uses the `SyncResult.enrichment_scope` to selectively re-embed. | `reembed.rs` |
| **7. Release/Drain** | Lock is released (RAII `Drop` impl). `PendingSync::claim()` atomically renames `sync.pending` to `sync.processing`; caller re-syncs then calls `complete_processing()`. | `lock.rs`, `pending.rs` |

## Key Public Types and Functions

### `SyncManager` (`sync.rs`)
Main orchestrator. Created with `SyncManager::new(base_path)` or `::with_config(base_path, config)`.

- `sync(conn, queries)` -- Full sync (filesystem walk detection).
- `sync_with_codegraph_dir(conn, queries, codegraph_dir)` -- Full sync with lock support.
- `sync_files(conn, queries, file_paths)` -- Sync specific files only.

Returns `SyncResult` containing:
- `stats` (`SyncStats`) -- files/nodes added/modified/deleted, errors.
- `enrichment_scope` (`SelectiveScope`) -- which nodes/files need re-enrichment.
- `deleted_node_ids` -- old node IDs from modified+deleted files (consumer must filter against new IDs to find truly deleted ones).
- `pre_delete_impact` (`ImpactCapture`) -- neighbor and sibling IDs captured before deletion.
- `changed_file_paths` -- all paths that changed.

### `ChangeDetector` (`change_detector.rs`)
Filesystem-walk change detection. Compares SHA256 hashes of files on disk against `files` table.

- `detect_changes(conn, queries)` -- Walk entire tree, return `Vec<FileChange>`.
- `detect_changes_for_files(conn, queries, file_paths)` -- Check specific files only.
- `summarize_changes(changes)` -- Returns `(added, modified, deleted)` counts.

### `GitDiffDetector` (`git_diff.rs`)
Git-based change detection. Faster than filesystem walk for hook-triggered syncs.

- `detect_changes(checkpoint)` -- Merges committed diffs (`checkpoint..HEAD`), working tree changes, and untracked files. Filters by supported language and exclude patterns.
- Handles renames (`R` status) as delete-old + add-new. Copies (`C`) as add-new only.
- Parses NUL-delimited `--name-status -z` output.

### `EdgeSnapshot` / `EdgeDiff` (`edge_diff.rs`)
Snapshot-and-diff for graph edges.

- `EdgeSnapshot::capture(conn)` -- Snapshot all edges from DB.
- `EdgeSnapshot::capture_for_files(conn, file_paths)` -- Snapshot edges involving specific files.
- `EdgeDiff::compute(before, after)` -- Returns added/removed edges and set of affected node IDs.

### `ImpactCapture` (`impact.rs`)
Targeted pre-delete impact capture. Must be called BEFORE nodes are deleted (while edges still exist).

- `ImpactCapture::capture(conn, old_node_ids)` -- Returns `affected_ids` (edge neighbors via calls/extends/implements/references) and `sibling_ids` (share a Contains parent). Chunked at 500 IDs per query. Excludes the input IDs themselves from results.
- `merge(other)` -- Combine two captures.

### `SelectiveScope` (`selective.rs`)
Determines re-enrichment scope after sync.

- `from_changed_files(files)` -- File-level scope only.
- `from_changed_files_with_deps(conn, files)` -- Includes nodes from `enrichment_deps` table.
- `from_changed_files_with_cascade(conn, files, cascade_depth)` -- Transitive dependency cascade (depth N walks N levels through `enrichment_deps` -> node -> file -> deps).
- `from_import_diff(old_imports, new_imports)` -- Detects new imports.
- `should_enrich(file)` / `should_enrich_node(node_id)` -- Membership checks.
- Handles cycles in dependency graph (visited-file set prevents infinite loops).

### `IndexLock` (`lock.rs`)
File-based exclusive lock (`<codegraph_dir>/index.lock`).

- `acquire(codegraph_dir)` -- Create lock file with `create_new`. Writes PID. Breaks stale locks (mtime > 5 min).
- `try_acquire_or_pending(codegraph_dir, hook_name)` -- Returns `Some(lock)` on success, `None` if pending was written (for hooks).
- `refresh()` -- Touch mtime to prevent stale detection during long syncs.
- `release()` -- Explicit release. Also releases on `Drop`. Verifies PID ownership before deleting lock file.

### `PendingSync` (`pending.rs`)
Coalesces hook events that arrive during an active sync.

- `write(hook_name)` -- Atomic write-then-rename of `sync.pending` (JSON with hook name + timestamp). Only accepts valid hook names.
- `claim()` -- Atomically renames `sync.pending` to `sync.processing`, returns hook name.
- `complete_processing()` -- Removes `sync.processing` marker.
- `cleanup()` -- Removes both markers (error recovery).
- Multiple writes before a claim are coalesced (latest wins).

### `GitHooksManager` (`git_hooks.rs`)
Manages git hook installation/uninstallation.

- `new(repo_path, force)` -- Uses `git rev-parse --git-path hooks` to find hooks dir (works with worktrees and `core.hooksPath`).
- `install_all()` / `uninstall_all()` -- Manages all four hooks.
- `install_hook(name)` / `uninstall_hook(name)` -- Per-hook operations.
- `is_installed()` / `list_installed()` -- Status checks.

### Checkpoint (`checkpoint.rs`)
Stores last-synced HEAD SHA in the `metadata` table.

- `read_last_head(conn, queries)` / `write_last_head(conn, queries, sha)` -- Read/write `sync.last_head`.
- `write_last_timestamp(conn, queries)` -- Write `sync.last_timestamp`.
- `get_git_head(repo_root)` -- Shell out to `git rev-parse HEAD`.

### Reembed triggers (`reembed.rs`)
- `check_reembed_triggers(conn, queries, config)` -- Returns `Vec<ReembedReason>`. Checks schema version, config hash (SHA256 of JSON), model hash, force flag, and first-embed.
- `should_full_reembed(conn, queries, config)` -- Convenience boolean wrapper.
- `record_embed_metadata(conn, queries, config)` -- Writes schema version, config hash, model hash to `metadata` table after successful embed.

## Git Hooks Supported

| Hook | When it fires |
|------|--------------|
| `post-commit` | After a commit |
| `post-checkout` | After `git checkout` / `git switch` |
| `post-merge` | After `git merge` / `git pull` |
| `post-rewrite` | After `git rebase` / `git commit --amend` |

Hook script behavior:
1. Checks `git rev-parse --is-inside-work-tree` and `.codegraph/` exists.
2. Chains to original hook if `.codegraph-orig` backup exists (preserves exit code).
3. Runs `codegraph sync --hook <name>` in background, logging to `.codegraph/sync.log`.

Install safeguards:
- Detects Husky (`.husky/`) and Lefthook (`lefthook.yml` / `.lefthook.yml`). Refuses install unless `--force`.
- Refuses if `.codegraph-orig` backup already exists (stale state from previous incomplete install/uninstall).
- Existing non-CodeGraph hooks are backed up to `<hook>.codegraph-orig` and restored on uninstall.
- Re-installing over an existing CodeGraph hook updates in place (no duplicate backup).

## Locking System

- **Lock file**: `<codegraph_dir>/index.lock`
- **Contents**: PID of the holder (for debugging and ownership verification).
- **Stale detection**: mtime > 5 minutes = stale. Automatically broken on next acquire.
- **Heartbeat**: `lock.refresh()` touches mtime. Call periodically during long syncs.
- **PID verification on release**: Lock file is only deleted if its PID matches `std::process::id()`. Prevents removing a lock stolen by another process.
- **Hook contention path**: `try_acquire_or_pending()` -- if lock is held, writes `sync.pending` and returns `None` (no blocking, no error).
- **RAII**: `IndexLock` releases on `Drop`.

## Checkpoint System

Two metadata keys in SQLite `metadata` table:
- `sync.last_head` -- SHA of HEAD at last successful sync. Used by `GitDiffDetector` as the `checkpoint` argument.
- `sync.last_timestamp` -- Unix timestamp of last sync.

Both are written after a successful sync. Read before the next sync to determine the diff range.

## Reembed Triggers

A full re-embed of all nodes is triggered when any of these change:
- **Schema version** (`embedding_schema_version` in metadata) -- DB migration occurred.
- **Config hash** (`embedding_config_hash`) -- Embedding text config changed (SHA256 of JSON, truncated to 16 hex chars).
- **Model hash** (`embedding_model_hash`) -- Different embedding model.
- **Force flag** -- User explicitly requested.
- **First embed** -- No previous metadata exists.

When none of these trigger, incremental re-embedding uses `SelectiveScope`.

## Gotchas

- **Parse-first safety**: `process_modify` parses new content BEFORE deleting old nodes. If extraction fails, old data is preserved.
- **ImpactCapture must run pre-delete**: `ImpactCapture::capture()` queries edges that will be CASCADE-deleted when nodes are removed. Call it before `delete_nodes_by_file`.
- **EdgeSnapshot vs ImpactCapture**: `EdgeSnapshot` captures ALL edges (expensive). `ImpactCapture` is targeted to changed nodes only (proportional to change size). Prefer `ImpactCapture` on the hot path.
- **deleted_node_ids includes recreated nodes**: `SyncResult.deleted_node_ids` for modified files includes IDs that may exist again with new content. Consumer must filter against new node IDs.
- **Pending coalescing is lossy**: Multiple hook events between syncs collapse to the last one. The hook name is recorded but earlier events are overwritten.
- **GitDiffDetector skips files that don't exist on disk**: If git reports a file as changed but it's missing from the working tree, it's silently skipped (not treated as deleted).
- **ChangeDetector skips `Language::Unknown` files**: Only files with recognized extensions are tracked. Adding a new language to `Language::from_extension` may cause a wave of "Added" changes on next sync.
- **Lock stale timeout is 5 minutes**: Long-running syncs must call `lock.refresh()` to avoid being broken by a concurrent process.
- **`continue_on_error` defaults to `true`**: Errors for individual files are collected in `SyncStats.errors` but do not abort the sync.
