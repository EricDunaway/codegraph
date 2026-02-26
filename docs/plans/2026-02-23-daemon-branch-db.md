# Daemon Mode & Branch-Based Databases

**Date:** 2026-02-23
**Status:** Design
**Depends on:** `2026-02-22-sync-redesign.md` (Milestones 1-3 must land first)
**Crates affected:** `codegraph-core`, `codegraph-db`, `codegraph-sync`, `codegraph-cli`, `codegraph-mcp`, new `codegraph-daemon` crate

## Purpose

Add an optional per-project daemon (`codegraphd`) and branch-based database support to CodeGraph. The daemon provides persistent ONNX sessions, warm SQLite connections, filesystem watching, and event coalescing. Branch databases give each git branch its own index, with intelligent donor selection that copies the closest existing database instead of re-indexing from scratch.

## Non-Goals

- Global daemon (single daemon managing all projects)
- Real-time collaborative editing support
- Remote/networked daemon access
- Replacing the 7-phase sync pipeline (daemon is a hybrid accelerator on top of it)

## Design Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Daemon scope | Per-project (one per `.codegraph/`) | Simpler ownership, matches per-project data model |
| Branch DB storage | Direct open at branch path, no active mirror | Eliminates copy-on-switch durability holes |
| Branch DB key | SHA256(canonical refname)[0:12] + manifest | Avoids filesystem path issues with branch names |
| Donor selection | Merge-base ancestry distance | Minimizes commits-to-sync after copy |
| DB copy method | SQLite backup API | Online, consistent, handles WAL correctly |
| Cache invalidation | `db_generation` counter in metadata | Bumped every write batch, not just branch switch |
| MCP interaction | Direct read connection + reopen on generation change | Read latency parity with today |
| Daemon lifecycle | Auto-start on first command, idle timeout | Good UX without manual management |
| IPC transport | Unix socket (`.codegraph/daemon.sock`) | Local-only, low overhead, standard |
| Feature gate | `branch_databases` in config.json | Opt-in, backward compatible |

## Branch-Based Databases

### Concept

When branch mode is enabled, each git branch gets its own SQLite database. On branch switch, CodeGraph opens the branch-specific database instead of a shared one. When a branch has no database yet, the closest existing database (by git ancestry) is copied as a starting point, then only the diff needs to be synced.

### DB Path Resolution

Single function `CodeGraphConfig::resolve_db_path()` used by all consumers:

```
Branch mode OFF (default):
  .codegraph/codegraph.db

Branch mode ON:
  .codegraph/branches/<key>/codegraph.db
  where key = SHA256(canonical_refname)[0:12]
```

**Canonical refname:** Output of `git symbolic-ref HEAD` (e.g., `refs/heads/feature/auth`). For detached HEAD, falls back to commit SHA.

**Config wiring fix required:** Multiple call sites use `CodeGraphConfig::new()` which hardcodes the DB path:
- `CodeGraph::init()` (`codegraph.rs:44`)
- `CodeGraph::open()` (`codegraph.rs:73`)
- `serve_mcp` in CLI (`commands.rs:390`)

All must switch to `CodeGraphConfig::load()` to read `branch_databases` from config.json and resolve the correct DB path.

### Manifest

```json
// .codegraph/branches/manifest.json
{
  "schema_version": 1,
  "branches": {
    "a1b2c3d4e5f6": {
      "refname": "refs/heads/main",
      "head_sha": "abc123...",
      "source_sha": null,
      "source_key": null,
      "db_schema_version": 5,
      "config_hash": "abc123def456...",
      "created_at": "2026-02-23T10:00:00Z",
      "last_used_at": "2026-02-23T18:30:00Z",
      "pinned": true
    },
    "f6e5d4c3b2a1": {
      "refname": "refs/heads/feature/auth",
      "head_sha": "def456...",
      "source_sha": "abc123...",
      "source_key": "a1b2c3d4e5f6",
      "db_schema_version": 5,
      "config_hash": "abc123def456...",
      "created_at": "2026-02-23T14:00:00Z",
      "last_used_at": "2026-02-23T18:30:00Z",
      "pinned": false
    }
  }
}
```

Manifest is updated atomically (write tmp + rename).

### Donor Selection Algorithm

When a branch has no database:

1. List all branches with DBs from manifest
2. Filter candidates: same `db_schema_version`, same config hash (SHA256 of serialized indexing config: exclude patterns, max_file_size, embedding config, resolver settings)
3. For each candidate, check ancestry: `git merge-base --is-ancestor <candidate_head> <current_head>`
4. Among ancestors: pick the one with minimum `git rev-list --count <candidate_head>..<current_head>` (fewest commits to sync)
5. If no ancestor found: compute symmetric distance via merge-base, pick minimum
6. If best distance exceeds threshold (default 1000 commits): skip donor, do fresh index
7. Default branch DB is always kept as fallback donor

**After donor copy:**
- Rewrite `sync.last_head` in metadata to donor's `head_sha`
- Clear `sync.failed` and `sync.pending` state
- Run incremental sync (git diff from donor head to current HEAD)

### Detached HEAD Handling

- Detached HEAD (no symbolic ref): use commit SHA as key
- During rebase (`.git/rebase-merge` or `.git/rebase-apply` exists): keep current DB open, do not switch
- During merge conflict (`.git/MERGE_HEAD` exists): keep current DB, do not switch
- Ephemeral commit-keyed DBs: subject to aggressive pruning (1 hour TTL)

### Branch Identity in Worktrees

- Each worktree has its own `.codegraph/` directory (worktree-local)
- Branch DB keys are scoped to the `.codegraph/` they live in
- Daemon is per-worktree root, not per-repository
- Pruning checks `git worktree list` before deleting any branch DB
- **Hook prerequisite:** Current `GitHooksManager` assumes `.git` is a directory, which breaks in worktrees where `.git` is a file. The sync-redesign plan (Milestone 3, step 13) already requires using `git rev-parse --git-path hooks/<name>` for hook discovery. That fix must land before branch DB worktree support works.

### MCP/CLI Path Fix

Current code derives repo root from DB path using parent traversal:
- `McpServer` uses `parent().parent()` to find repo root
- `get_last_sync_time` hardcodes `.codegraph/codegraph.db`
- CLI `status` assumes fixed DB path

Fix: store `repo_root` explicitly in `CodeGraphConfig` (already exists as `root` field). All consumers use `config.root` instead of deriving from DB path.

---

## Daemon Architecture

### Component: `codegraphd`

A per-project long-running process providing:

1. **Persistent ONNX session** — avoids ~100ms tokenizer/session init per sync
2. **Warm SQLite connection** — avoids cold open + pragma setup per sync
3. **Filesystem watcher** — FSEvents (macOS) / inotify (Linux) for real-time edit tracking
4. **Event coalescing** — in-memory debounce replaces file-based `sync.pending`
5. **IPC server** — Unix socket for CLI, MCP, and hooks to communicate with daemon

### Daemon State Machine

```
                    ┌──────────┐
         ┌─────────│ Starting │
         │         └────┬─────┘
         │              │ init complete
         │              v
         │         ┌──────────┐
         │    ┌────│  Ready   │◄──────────────┐
         │    │    └────┬─────┘               │
         │    │         │ event received       │ sync/switch done
         │    │         v                      │
         │    │    ┌──────────┐    branch   ┌──────────┐
         │    │    │ Syncing  │────change──>│Switching │
         │    │    └────┬─────┘             └────┬─────┘
         │    │         │                        │
         │    │         └────────────────────────┘
         │    │
         │    │ idle timeout / SIGTERM / shutdown command
         │    v
         │  ┌──────────────┐
         └─>│ ShuttingDown │
            └──────────────┘
```

### Daemon Lifecycle

**Auto-start:** When `codegraph sync`, `codegraph status`, or MCP server starts, check for `daemon.sock`. If not present and `daemon.enabled` is true in config, fork daemon process. Branch mode alone does NOT trigger daemon auto-start — branch DBs work without daemon via inline `BranchDbManager` calls.

**Single-instance enforcement:**
1. Acquire `daemon.lock` (PID file with flock)
2. If lock held by live process: connect as client instead
3. If lock held by dead process: steal lock, clean up stale socket

**Idle timeout:** Configurable (default 30 minutes). Reset on any IPC activity or sync event. On timeout: checkpoint WAL, close connections, remove socket, release lock, exit.

**Shutdown:** `codegraph daemon stop` or SIGTERM. Drain in-flight sync, checkpoint, clean up.

### IPC Protocol

JSON messages over newline-delimited Unix socket stream.

**Requests (client to daemon):**
```json
{"id": 1, "method": "sync", "params": {"force": false}}
{"id": 2, "method": "status"}
{"id": 3, "method": "health"}
{"id": 4, "method": "shutdown"}
{"id": 5, "method": "prune", "params": {"dry_run": true}}
```

**Responses:**
```json
{"id": 1, "result": {"had_changes": true, "stats": {...}}}
{"id": 2, "error": {"code": -1, "message": "sync in progress"}}
```

**Events (daemon to subscribed clients):**
```json
{"event": "generation_changed", "data": {"generation": 42, "branch": "refs/heads/main", "db_path": "..."}}
{"event": "sync_complete", "data": {"stats": {...}}}
{"event": "sync_failed", "data": {"error": "..."}}
```

**Protocol versioning:** First message from client includes `{"protocol": 1}`. Daemon rejects incompatible versions.

**Reconnect:** Clients should reconnect on socket error, re-subscribe to events, and re-check `db_generation`.

### Filesystem Watcher

**macOS:** FSEvents (via `notify` crate with fsevent backend)
**Linux:** inotify (via `notify` crate)

**Event flow:**
1. Watch project root recursively (respecting exclude patterns)
2. Debounce: collect events for 200ms after last change
3. Filter to supported file extensions
4. Feed file list into Phase 1 (external file list mode) of sync pipeline
5. If debounce window exceeds 5 seconds of continuous changes, flush and process

**Overflow handling:** If watcher reports overflow (too many events), fall back to git diff or full hash scan on next sync.

**Exclude patterns:** Same as `CodeGraphConfig.exclude_patterns`. Watch setup skips excluded directories entirely.

### Event Coalescing

In-memory queue replaces file-based `sync.pending`:

```
Event arrives -> Debounce timer (200ms)
                    |
                    v
              Coalesce into batch
                    |
                    v
              If syncing: queue for after current sync
              If idle: start sync immediately
```

**Backpressure:** If queue exceeds 10,000 pending files, collapse to "full sync needed" marker.

---

## `db_generation` Protocol

### Definition

Monotonic counter stored in metadata table:
```sql
INSERT OR REPLACE INTO metadata(key, value) VALUES('db_generation', ?);
```

### Increment Rules

- Incremented inside every write transaction that modifies nodes, edges, files, or vectors
- Branch switch: incremented after opening new DB
- Fresh index: incremented after completion
- Never decremented

### Reader Contract

Any process with a read connection (MCP, CLI status):
1. On open: resolve current branch DB path, read `db_generation`, store both locally
2. Periodically (or on IPC event): re-resolve the branch DB path via `resolve_db_path()`
3. If DB path changed (branch switch detected): close old connection, open new DB, reset caches
4. If same path but `db_generation` changed: drop in-process caches (`QueryBuilder` cached results)
5. Frequency: every 5 seconds for polling, or immediately on IPC notification from daemon

**Without daemon:** MCP must poll both the DB path (re-resolve via git) and `db_generation`. This is the only way to detect branch switches without daemon events. The 5-second poll interval means MCP may serve stale data for up to 5 seconds after a branch switch — acceptable for a no-daemon configuration.

---

## Configuration

Added to `.codegraph/config.json`:

```json
{
  "branch_databases": false,
  "daemon": {
    "enabled": false,
    "idle_timeout_minutes": 30,
    "fs_watch": true,
    "debounce_ms": 200
  },
  "pruning": {
    "max_branch_dbs": 10,
    "ttl_days": 7,
    "detached_ttl_hours": 1
  }
}
```

`branch_databases` and `daemon.enabled` are independent. Branch DBs work without daemon. Daemon works without branch DBs (just provides persistent ONNX + FS watch).

---

## Security

- **Socket permissions:** `daemon.sock` created with mode 0600 (owner-only). Peer credential check via `SO_PEERCRED` (Linux) / `LOCAL_PEERCRED` (macOS).
- **Branch name sanitization:** Never use raw refnames in filesystem paths. Always hash. Manifest maps hash to refname.
- **IPC input validation:** All method names allowlisted. Path parameters validated against project root. No arbitrary command execution.
- **Daemon lock:** PID file with advisory flock prevents concurrent daemons.
- **DB copy security:** Donor compatibility checked (schema version, full config hash) before backup API call. Incompatible donors rejected.

---

## CLI Changes

```
codegraph daemon start [path]    # Start daemon for project
codegraph daemon stop [path]     # Stop daemon
codegraph daemon status [path]   # Show daemon health
codegraph prune [path]           # Prune old branch DBs
codegraph prune --dry-run [path] # Show what would be pruned
```

Existing commands updated:
- `codegraph sync` — connects to daemon if available, falls back to direct
- `codegraph status` — shows branch DB info and daemon status
- `codegraph init` — adds `branch_databases`/`daemon` to config template

### Pruning Safety

Pruning must coordinate with active consumers to avoid deleting open databases:

1. **If daemon is running:** prune requests go through daemon IPC. Daemon checks its active DB and in-flight sync before deleting.
2. **If no daemon:** prune acquires `IndexLock` and checks manifest `last_used_at` to avoid recently-active DBs. Cannot guarantee MCP isn't reading, so:
   - Rename DB to `.deleting` suffix first (atomic rename)
   - MCP will get SQLITE_ERROR on next read attempt and trigger re-resolve
   - Delete `.deleting` file after a short delay (1 second)
3. **Never prune:** current branch DB, default branch DB, pinned branches, any branch checked out in a worktree

---

## Implementation Order

### Prerequisites (from sync-redesign plan)

- Sync-redesign Milestone 1: process_modify safety, index_all clear
- Sync-redesign Milestone 2: scoped resolution, edge dedup, pre-delete capture
- Sync-redesign Milestone 3: hook operability, git-diff detection, lock scope

### Milestone 1: Branch Database Support (no daemon)

1. **Config wiring** — Switch `CodeGraph::init/open` to use `CodeGraphConfig::load()`, add `branch_databases` field (`codegraph-core`)
2. **`resolve_db_path()`** — Branch-aware path resolution with git integration (`codegraph-core`)
3. **`BranchDbManager`** — Manifest CRUD, donor selection, SQLite backup copy, metadata rewrite (`codegraph-sync` or new `codegraph-branch` crate)
4. **MCP path fix** — Use `config.root` instead of DB path traversal (`codegraph-mcp`)
5. **CLI integration** — `codegraph prune`, branch info in `codegraph status` (`codegraph-cli`)
6. **Hook integration** — post-checkout triggers branch DB switch in sync path (`codegraph-sync`)
7. **Tests** — donor selection, manifest CRUD, branch switch, detached HEAD, worktree isolation

### Milestone 2: Daemon Core

8. **`codegraph-daemon` crate** — daemon binary, PID/lock management, state machine
9. **IPC server** — Unix socket, JSON protocol, request/response/events
10. **Persistent ONNX** — Hold loaded `TextEmbedder` across syncs with idle-unload policy
11. **Persistent SQLite** — Hold `DatabaseConnection` + `QueryBuilder`, reopen on branch switch
12. **`db_generation` protocol** — Metadata counter, write-batch increment, reader polling/notification
13. **CLI daemon commands** — `codegraph daemon start/stop/status`
14. **Auto-start** — Fork daemon from `codegraph sync`/MCP if enabled and not running
15. **Tests** — single-instance lock, IPC roundtrip, auto-start/idle timeout, generation protocol

### Milestone 3: Filesystem Watcher

16. **FS watcher integration** — `notify` crate, exclude patterns, overflow fallback (`codegraph-daemon`)
17. **Debounce + coalescing** — In-memory event queue, backpressure (`codegraph-daemon`)
18. **Wire to sync pipeline** — Feed FS events as external file list into Phase 1 (`codegraph-daemon`)
19. **Tests** — debounce behavior, overflow fallback, exclude patterns

### Future

20. **Migration across branch DBs** — Run schema migrations on all known branch DBs (lazy, on first open)
21. **Branch DB size reporting** — `codegraph status --branches` shows per-branch DB sizes
22. **Pre-commit staged preview** — Optional pre-compute from `git diff --cached`

---

## Performance Expectations

| Scenario | Without Daemon | With Daemon |
|----------|---------------|-------------|
| Single file edit sync | ~20ms (sync-redesign) + 100ms ONNX load | ~20ms (ONNX pre-loaded) |
| Branch switch (DB exists) | ~50ms (open + sync diff) | ~30ms (reopen + sync) |
| Branch switch (new branch, donor copy) | ~500ms (backup API + sync diff) | ~500ms (same, but subsequent syncs faster) |
| Branch switch (no donor, fresh index) | ~5s+ (full index) | ~5s+ (same) |
| Edit detection latency | Hook-only (~1s after git op) | <500ms (FS watch + debounce) |
| Idle memory (daemon) | N/A | ~50-100MB (ONNX session + SQLite cache) |

---

## Affected Files

| File | Changes |
|------|---------|
| `crates/codegraph-core/src/config.rs` | `branch_databases`, `daemon` config, `resolve_db_path()` |
| `crates/codegraph-core/src/codegraph.rs` | Use `load()` instead of `new()`, branch-aware open, `db_generation` writes |
| `crates/codegraph-db/src/connection.rs` | `db_generation` increment helper, busy timeout config |
| `crates/codegraph-db/src/queries.rs` | Generation-aware cache invalidation |
| `crates/codegraph-sync/src/sync.rs` | Branch switch integration, donor copy trigger |
| `crates/codegraph-mcp/src/server.rs` | Use `config.root`, generation-based reopen |
| `crates/codegraph-mcp/src/git.rs` | Remove hardcoded DB path in `get_last_sync_time` |
| `crates/codegraph-cli/src/main.rs` | `daemon` and `prune` subcommands |
| `crates/codegraph-cli/src/commands.rs` | Branch info in status, daemon auto-start |
| NEW `crates/codegraph-daemon/` | Daemon binary, IPC, FS watcher, state machine |

---

## Appendix: Codex Review Summary

Developed through 7 iterative Claude-Codex rounds. Key corrections from Codex:

1. Active-file mirroring (copy codegraph.db on every switch) has durability holes and is over-engineered. Eliminated in favor of direct branch DB path resolution.
2. `db_generation` must increment on every write batch, not just branch switch, to prevent stale reads from MCP.
3. MCP path assumptions (`parent().parent()` repo derivation, hardcoded DB path in `get_last_sync_time`) break with branch DB paths.
4. QueryBuilder has in-process caching that goes stale on external writes — needs generation-based invalidation.
5. SQLite WAL copy requires checkpoint or backup API, not raw file copy.
6. Donor copy must verify full config compatibility (not just schema + embedding), including excludes, max_file_size, and resolver settings.
7. Branch name sanitization must use hashing, not string manipulation, to avoid filesystem collisions.
8. Config wiring fix needed: `init/open` AND `serve_mcp` use `new()` not `load()`, so config file settings are currently ignored.
9. Fix sync correctness (delete-before-parse, lock scope) from sync-redesign before daemonizing.
10. Ship in phases: branch DB correctness first, daemon second, FS watcher third.
11. `db_generation` alone cannot detect DB path switches without daemon — readers must also poll the resolved DB path via git.
12. Auto-start should only trigger when `daemon.enabled` is true, not on `branch_databases` alone.
13. Pruning must coordinate with active connections — use rename-to-tombstone pattern to avoid deleting open SQLite files.
14. Worktree hook support requires `git rev-parse --git-path` fix from sync-redesign plan (current code assumes `.git` is a directory).
