# codegraph-lsp

LSP integration crate that enriches code nodes with inferred types and resolved definitions by communicating with language servers over JSON-RPC stdio.

## Supported Languages

| Language | LSP Server | Enricher | `language_id` |
|----------|-----------|----------|---------------|
| TypeScript/JS | `typescript-language-server --stdio` | `TypeScriptEnricher` | `"typescript"` |
| Rust | `rust-analyzer` | `RustEnricher` | `"rust"` |
| Python | `pyright-langserver --stdio` | `PythonEnricher` | `"python"` |
| Go | `gopls` | `GoEnricher` | `"go"` |
| Dart | `dart language-server` | `DartEnricher` | `"dart"` |

## Architecture

### LspEnricher Trait (`src/enricher.rs`)

Async trait defining the interface every language enricher must implement:

```rust
#[async_trait]
pub trait LspEnricher: Send + Sync {
    async fn start(&mut self, workspace_root: &Path) -> Result<(), LspError>;
    async fn hover(&self, file: &Path, line: u32, column: u32) -> Result<Option<HoverResult>, LspError>;
    async fn definition(&self, file: &Path, line: u32, column: u32) -> Result<Option<DefinitionResult>, LspError>;
    async fn shutdown(&mut self) -> Result<(), LspError>;
    async fn is_ready(&self) -> bool;
    fn language(&self) -> Language;
}
```

### BaseEnricher (`src/base.rs`)

Shared implementation that all language enrichers embed via composition (not inheritance). Handles:

- Spawning and initializing the LSP server via `LspClient`
- Executing hover/definition queries with concurrent access (client stored in `Arc<Mutex<Option<Arc<LspClient>>>>` -- the Mutex only guards init/shutdown, not individual requests)
- Parsing hover responses via a language-specific `TypeParser` function (`fn(&str) -> Option<String>`)
- Extracting documentation from hover markdown content
- Parsing `GotoDefinitionResponse` (scalar, array, link variants) into `DefinitionResult`

Each language enricher provides its own `parse_type_declaration` / `parse_type_annotation` static method that understands that language's hover output format (e.g., `const foo: string` for TS, `let x: i32` for Rust, `def get_name() -> str:` for Python).

### LspClient (`src/client.rs`)

Fully async JSON-RPC client that:

- Spawns the LSP server as a child process (`kill_on_drop(true)`)
- Runs a background tokio task (`response_reader_task`) that reads stdout, parses `Content-Length` framed messages, and dispatches responses to waiting `oneshot` channels by request ID
- Supports concurrent in-flight requests (multiple hover/definition calls to the same server)
- Sends `initialize` + `initialized`, `textDocument/hover`, `textDocument/definition`, `shutdown` + `exit`
- Default timeout: 30s per request, 5s for shutdown
- 10MB max response size guard

### LspServerManager (`src/lifecycle.rs`)

Lifecycle wrapper around `LspClient` providing:

- **Lazy spawn**: server started only on first `ensure_started()` call
- **Keep-alive**: stays running across batch operations
- **Retry with backoff**: `ensure_started_with_retry(path, max_retries)` with exponential backoff (100ms, 200ms, 400ms...)
- **Graceful shutdown**: sends shutdown/exit, waits, then force-kills on timeout

### LspSyncBridge (`src/sync_bridge.rs`)

Bridges async LSP enrichers into sync code paths (indexer, graph traversal). Creates a dedicated tokio `Runtime` (single-thread or multi-thread) and exposes `block_on()` and `spawn()`.

```rust
let bridge = LspSyncBridge::new()?;          // single-thread
let bridge = LspSyncBridge::new_multi_thread(4)?; // multi-thread
let result = bridge.block_on(enricher.hover(file, 10, 5));
```

### Batch Enrichment (`src/batch.rs`)

- `enrich_node()` -- enrich a single node (hover + definition)
- `enrich_batch()` -- enrich many nodes concurrently (default concurrency: 50)
- `enrich_batch_with_concurrency()` -- custom concurrency limit (panics if 0)
- Uses `futures::stream::buffer_unordered` for bounded parallelism
- Tracks cross-file dependencies in `NodeEnrichmentResult.deps_to_record` (for `enrichment_deps` table)

Key types:
- `EnrichmentRequest` -- specifies node_id, file_path, optional hover/definition positions
- `NodeEnrichmentResult` -- inferred_type, documentation, resolved_import_path, deps_to_record
- `BatchEnrichmentResult` -- results vec with success/failure counts, `.successful()` / `.failed()` iterators

## Enrichment Data Produced

| Query | Result Type | Data |
|-------|------------|------|
| `hover` | `HoverResult` | `inferred_type: Option<String>`, `documentation: Option<String>` |
| `definition` | `DefinitionResult` | `file_path: String`, `line: u32`, `column: u32` |

- **Inferred types**: extracted from hover markdown by language-specific parsers (e.g., `const foo: Promise<User[]>` yields `"Promise<User[]>"`)
- **Documentation**: raw markdown from hover contents
- **Definition locations**: resolved file path + position (for import resolution); cross-file definitions recorded as enrichment dependencies

## Encoding Helpers (`src/encoding.rs`)

LSP uses UTF-16 positions; Rust uses UTF-8 byte offsets. Four conversion functions:

| Function | Direction |
|----------|-----------|
| `byte_to_utf16(text, byte_offset)` | Rust byte offset -> LSP UTF-16 column |
| `utf16_to_byte(text, utf16_offset)` | LSP UTF-16 column -> Rust byte offset (returns `None` if out of bounds) |
| `position_to_byte_offset(text, line, col_utf16)` | LSP (line, col) -> byte offset |
| `byte_offset_to_position(text, byte_offset)` | byte offset -> LSP (line, col) |

## Error Types (`src/error.rs`)

`LspError` enum: `StartFailed`, `NotReady`, `Timeout`, `ServerError { code, message }`, `InvalidResponse`, `Io`, `Json`, `ServerShutdown`, `UnsupportedLanguage`, `FileNotFound`, `PositionOutOfBounds { line, column }`.

## Gotchas

- **UTF-16 encoding**: Always use the encoding helpers when converting between tree-sitter byte offsets and LSP positions. Emoji and CJK characters cause byte vs UTF-16 offset divergence.
- **Concurrency model**: `BaseEnricher` stores the client as `Arc<Mutex<Option<Arc<LspClient>>>>`. The outer Mutex only protects init/shutdown; the inner Arc allows concurrent LSP requests without holding any lock. Do not hold the Mutex across LSP calls.
- **`LspSyncBridge` cannot nest**: Calling `block_on` from within an existing tokio runtime will panic. Only use from truly synchronous code paths.
- **Server must be installed**: The crate spawns external LSP binaries. If the binary is not on `$PATH`, `LspClient::spawn` returns `LspError::StartFailed`.
- **`validate_lsp_config` only checks TS, Dart, Rust**: Python and Go configs are not validated in `lib.rs` (the enrichers themselves check `enabled` on `start()`).
- **Batch concurrency default is 50**: May need tuning for large repos or slow LSP servers. Set to 0 will panic.
- **`textDocument/didOpen` not sent**: The client does not send `didOpen` notifications. Some LSP servers may return empty results for files they haven't been told about. This works in practice because most servers fall back to reading from disk.
- **Shutdown race**: On drop, `LspClient` aborts the reader task and relies on `kill_on_drop(true)`. If the process lingers, the OS handle is cleaned up by the child process destructor.
