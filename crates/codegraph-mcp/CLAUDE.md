# codegraph-mcp

MCP (Model Context Protocol) server that exposes CodeGraph functionality over JSON-RPC on stdio. Clients send one JSON-RPC message per line; the server replies with one JSON-RPC message per line.

## Modules

| File | Purpose |
|------|---------|
| `src/lib.rs` | Re-exports `McpError`, `McpServer`, `McpTools` |
| `src/server.rs` | Stdio read loop, JSON-RPC dispatch, lifecycle (`initialize` / `shutdown`) |
| `src/protocol.rs` | Wire types: `JsonRpcRequest`, `JsonRpcResponse`, `ToolDefinition`, `ToolCallResult`, `ContentBlock` |
| `src/tools.rs` | All tool implementations + staleness logic |
| `src/error.rs` | `McpError` enum (wraps `DbError`, `GraphError`, `ContextError`, IO, JSON) |
| `src/git.rs` | `get_git_status`, `are_hooks_installed`, `get_last_sync_time` |

## Tools

All tools return `ToolCallResult { content: [ContentBlock::Text], is_error: bool }`. Output is human-readable Markdown text, not structured JSON.

### codegraph_search

Search symbols by name (FTS5). Also surfaces parent containers of matched symbols (e.g., searching "login" returns the containing `AuthService` class too).

| Param | Type | Required | Default |
|-------|------|----------|---------|
| `query` | string | yes | - |
| `limit` | integer | no | 20 |

Output line format: `- {kind} \`{qualified_name}\` ({file}:{line}) [id: {node_id}]`

### codegraph_context

Build relevant code context for an AI task. Tries semantic search (ONNX embeddings) first, falls back to FTS if embeddings are unavailable or return no hits.

| Param | Type | Required | Default |
|-------|------|----------|---------|
| `query` | string | yes | - |
| `max_tokens` | integer | no | 8000 |

Output: Markdown document with source code blocks, assembled by `ContextBuilder`.

### codegraph_node

Get full details for a single symbol: kind, file location, language, signature, docstring, and source code snippet.

| Param | Type | Required | Default |
|-------|------|----------|---------|
| `node_id` | string | yes | - |

Output: Markdown with heading, metadata list, documentation block, fenced code block.

### codegraph_file_nodes

List all symbols defined in a file.

| Param | Type | Required | Default |
|-------|------|----------|---------|
| `file_path` | string | yes | - |

Output line format: `- {kind} \`{name}\` (line {n}) [id: {node_id}]`

### codegraph_status

Index health: file count, node count, last sync time, git hook status, dirty file list.

No parameters.

Output: Markdown with stats. If dirty files exist, lists up to 10 and suggests `codegraph sync`.

### codegraph_callers

Find what calls a given function/method. Requires resolved edges (run full `codegraph index` first).

| Param | Type | Required | Default |
|-------|------|----------|---------|
| `node_id` | string | yes | - |

Output line format: `- {kind} \`{qualified_name}\` ({file}:{line}) [id: {node_id}]`

### codegraph_callees

Find what a given function/method calls. Requires resolved edges.

| Param | Type | Required | Default |
|-------|------|----------|---------|
| `node_id` | string | yes | - |

Output line format: same as callers.

### codegraph_impact

Impact radius analysis: direct and indirect dependents of a symbol, up to `max_depth`. Indirect list is capped at 10 items in output (with "... and N more" overflow).

| Param | Type | Required | Default |
|-------|------|----------|---------|
| `node_id` | string | yes | - |
| `max_depth` | integer | no | 3 |

Output: Markdown with total affected count, direct dependents list, indirect dependents list.

## Constants (`src/tools.rs`)

```rust
pub const TOOL_SEARCH: &str = "codegraph_search";
pub const TOOL_CONTEXT: &str = "codegraph_context";
pub const TOOL_CALLERS: &str = "codegraph_callers";
pub const TOOL_CALLEES: &str = "codegraph_callees";
pub const TOOL_IMPACT: &str = "codegraph_impact";
pub const TOOL_NODE: &str = "codegraph_node";
pub const TOOL_FILE_NODES: &str = "codegraph_file_nodes";
pub const TOOL_STATUS: &str = "codegraph_status";
```

## JSON-RPC Protocol

Protocol version: `2024-11-05`. Transport: newline-delimited JSON over stdio.

### Supported methods

| Method | Has `id`? | Purpose |
|--------|-----------|---------|
| `initialize` | yes | Handshake; returns `protocolVersion`, `capabilities`, `serverInfo`, `instructions` |
| `tools/list` | yes | Returns all `ToolDefinition` objects |
| `tools/call` | yes | Executes a tool; params: `{ name, arguments }` |
| `shutdown` | yes | Resets initialized state |
| `notifications/initialized` | no (notification) | Client ack after initialize |
| `notifications/cancelled` | no (notification) | Client cancels a request |

### Error codes (`protocol::error_codes`)

```
PARSE_ERROR     = -32700
INVALID_REQUEST = -32600
METHOD_NOT_FOUND = -32601
INVALID_PARAMS  = -32602
INTERNAL_ERROR  = -32603
```

### Server construction

`McpServer::new(db_path)` opens the SQLite database and derives the repo root by going two parents up from the db file (expects `.codegraph/codegraph.db` layout).

## Staleness Warnings

Every tool except `codegraph_context` and `codegraph_status` checks `git status --porcelain -uall` on each call. If any file referenced in the results has uncommitted changes (modified, untracked, or deleted), a warning is appended to the output:

- Single file: `"Stale data: \`{path}\` has uncommitted changes. Results may be outdated."`
- Multiple files: `"Stale data: {N} files have uncommitted changes. Results may be outdated."`

`codegraph_status` handles staleness differently -- it reports dirty file counts and lists up to 10 dirty paths as part of its normal output.

`codegraph_context` does not attach staleness warnings (it delegates to `ContextBuilder`).

## Manual Testing

Build the binary, then pipe JSON-RPC messages via heredoc:

```bash
# Build
cargo build -p codegraph-cli

# Send initialize + tool call to a real index
cat <<'EOF' | cargo run -p codegraph-cli -- serve --mcp --db /path/to/repo/.codegraph/codegraph.db
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}
{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}
{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"codegraph_search","arguments":{"query":"parse"}}}
{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"codegraph_status","arguments":{}}}
EOF
```

Each output line is a complete JSON-RPC response. Tool results are in `result.content[0].text`.

Run crate tests:

```bash
cargo test -p codegraph-mcp
```

## Server Instructions

The `initialize` response includes an `instructions` field with workflow guidance for AI agents. This covers tool hierarchy (start with `codegraph_context`), node ID flow, and prerequisites. Built by `McpServer::build_instructions()` in `server.rs`.

## Gotchas

- **Tool output is Markdown text, not JSON.** The `content` array contains `ContentBlock::Text` with human-readable Markdown. Callers that need structured data must parse the text.
- **`codegraph_callers` / `codegraph_callees` / `codegraph_impact` require resolved edges.** If you only ran extraction without resolution, these return empty results. Run a full `codegraph index` to populate `calls`/`imports`/`extends` edges.
- **Semantic search in `codegraph_context` silently falls back to FTS.** If the ONNX model is missing, the feature flag is off, or no vectors exist in the DB, it falls back without error. Check `codegraph_status` to verify vector availability.
- **`git status --porcelain -uall` runs on every tool call** (except `codegraph_context`). In very large repos this may add latency.
- **Server derives repo path from db path.** It assumes `db_path` is `<repo>/.codegraph/codegraph.db` and navigates two parents up. If the db is at a non-standard location, git status checks will target the wrong directory.
- **Notifications return `None`.** Messages without an `id` field are treated as notifications and produce no response on stdout. Make sure requests include an `id`.
- **`max_depth` on `codegraph_impact` is cast to `u32`**, not `usize`. Extremely large values will wrap.
- **Indirect dependents in impact output are capped at 10 displayed items.** The full list is computed but only the first 10 are printed.
