# codegraph-extraction

Tree-sitter based code extraction for CodeGraph. Parses source files into nodes, structural edges, and unresolved references that a downstream resolution crate converts into semantic edges.

## Two-Stage Extraction Model

**Stage 1 (this crate):** Tree-sitter AST parsing produces:
- **Nodes** (File, Function, Class, Method, Import, etc.) with code snippets, decorators, visibility
- **Contains edges** (structural parent-child: File->Class, Class->Method, File->Function)
- **UnresolvedReference records** with `reference_kind`:
  - `EdgeKind::Calls` -- from function/method bodies, skipping `BUILTIN_SYMBOLS`
  - `EdgeKind::Imports` -- from import statements
  - `EdgeKind::Extends` -- from inheritance/superclass clauses

**Stage 2 (codegraph-resolution crate):** Converts `UnresolvedReference` -> resolved `Edge` records.

Extraction does NOT create calls/imports/extends edges directly. It only emits `UnresolvedReference` entries in `ExtractionResult.unresolved_references`.

## Key Public Types and Functions

### Core extraction

```rust
// Tree-sitter extractor (config-driven, not per-language implementations)
pub struct TreeSitterExtractor { .. }
impl TreeSitterExtractor {
    pub fn new() -> Self;
    pub fn extract(&mut self, source: &str, file_path: &str, lang: Language)
        -> Result<ExtractionResult, ExtractionError>;
}

// Thread-safe wrapper around TreeSitterExtractor (Mutex-guarded)
pub struct ExtractorRegistry { .. }
impl ExtractorRegistry {
    pub fn new() -> Self;
    pub fn supports(&self, lang: Language) -> bool;
    pub fn supported_languages(&self) -> Vec<Language>;
    pub fn extract(&self, source: &str, file_path: &str, lang: Language)
        -> Result<ExtractionResult, ExtractionError>;
}

// Deterministic node ID: SHA-256 of "{file_path}:{kind}:{name}:{line}"
pub fn generate_node_id(file_path: &str, kind: NodeKind, name: &str, line: u32) -> NodeId;
```

### Orchestrator

```rust
pub struct ExtractionOrchestrator { .. }
impl ExtractionOrchestrator {
    pub fn new(root_dir: impl AsRef<Path>, config: Config) -> Result<Self, ExtractionError>;
    pub fn with_defaults(root_dir: impl AsRef<Path>) -> Result<Self, ExtractionError>;
    pub fn index_all<F: FnMut(IndexProgress)>(&self, on_progress: F)
        -> Result<IndexResult, ExtractionError>;
    pub fn extract_file(&self, file: &ScannedFile) -> Result<ExtractionResult, ExtractionError>;
    pub fn extract_from_source(&self, source: &str, file_path: &str, language: Language)
        -> Result<ExtractionResult, ExtractionError>;
    pub fn sync(&self, previous_hashes: &HashMap<String, String>,
        previous_paths: &[String], on_progress: F)
        -> Result<(SyncResult, Vec<FileExtractionResult>), ExtractionError>;
}
```

### File scanner

```rust
pub struct FileScanner { .. }
impl FileScanner {
    pub fn new(root_dir: impl AsRef<Path>, config: &Config) -> Result<Self, ExtractionError>;
    pub fn with_defaults(root_dir: impl AsRef<Path>) -> Result<Self, ExtractionError>;
    pub fn scan(&self) -> Result<ScanResult, ExtractionError>;
    pub fn scan_with_progress<F: FnMut(usize, &str)>(&self, on_progress: F)
        -> Result<ScanResult, ExtractionError>;
    pub fn get_changed_files(&self, previous_hashes: &HashMap<String, String>)
        -> Result<Vec<ScannedFile>, ExtractionError>;
    pub fn get_removed_files(&self, previous_paths: &[String]) -> Result<Vec<String>, ExtractionError>;
}

pub struct ScanResult { pub files: Vec<ScannedFile>, pub skipped_dirs: usize, pub skipped_files: usize }
pub struct ScannedFile { pub path: String, pub absolute_path: PathBuf, pub language: Language,
    pub size: u64, pub content_hash: String }
```

### Snippet extraction

```rust
pub const DEFAULT_MAX_LINES: usize = 50;
pub const TRUNCATION_MARKER: &str = "// ... truncated";

pub fn extract_code_snippet(source: &str, node: &Node, max_lines: usize) -> Option<String>;
pub fn extract_code_snippet_for_range(source: &str, start_line: u32, end_line: u32,
    max_lines: usize) -> Option<String>;
```

Lines are 1-indexed. Snippets beyond `max_lines` get truncated with `TRUNCATION_MARKER` appended.

### Test detection

```rust
pub fn is_test_file(path: &str, language: Language) -> bool;
pub fn find_test_names_for_symbol(source: &str, symbol_name: &str, language: Language) -> Vec<String>;
pub fn extract_imports(source: &str, language: Language) -> Vec<ImportInfo>;
pub fn associate_tests_via_imports(test_source: &str, source_file_path: &str,
    language: Language) -> Vec<String>;
pub fn find_tests_for_symbol(symbol_name: &str, source_file: &str, test_file: &str,
    test_source: &str, language: Language) -> TestAssociation;

pub struct ImportInfo { pub names: Vec<String>, pub path: String }
pub enum AssociationMethod { Convention, Import, None }
pub struct TestAssociation { pub test_names: Vec<String>, pub method: AssociationMethod }
```

`find_tests_for_symbol` uses a hybrid approach: convention-based file matching first (e.g., `user.test.ts` tests `user.ts`), then falls back to import analysis.

### Package/module detection

```rust
pub fn extract_package_name(project_root: &Path, file_path: &str) -> Option<PackageInfo>;
pub fn extract_package_name_nearest(file_path: &Path) -> Option<PackageInfo>;
pub fn extract_package_for_file(project_root: &Path, file_path: &str) -> Option<PackageInfo>;
pub fn is_workspace_root(project_root: &Path) -> bool;

pub struct PackageInfo { pub name: String, pub source: PackageSource }
pub enum PackageSource { PackageJson, CargoToml, PubspecYaml, GoMod, PyprojectToml,
    SetupPy, ComposerJson, DirectoryName }
```

Priority order: package.json > Cargo.toml > pubspec.yaml > go.mod > pyproject.toml > composer.json > directory name fallback.

`extract_package_for_file` handles monorepos by walking up from the file's directory to find the nearest sub-package manifest before the workspace root.

### Error extraction

```rust
pub fn extract_thrown_errors(source: &str, start_line: u32, end_line: u32,
    language: Language) -> Vec<String>;
```

Regex-based. Extracts `throw new X`, `Err(X)`, `raise X`, etc. Returns deduplicated, sorted error type names.

## Feature Flags for Language Support

```toml
[features]
default = ["all-languages"]
all-languages = ["lang-typescript", "lang-rust", "lang-php", "lang-dart", "lang-swift",
    "lang-graphql", "lang-hcl", "lang-bash", "lang-python", "lang-go", "lang-java",
    "lang-c", "lang-cpp", "lang-csharp", "lang-ruby"]
```

Each `lang-*` feature gates the tree-sitter grammar dependency. `ExtractorRegistry::supported_languages()` returns only languages whose features are enabled. `lang-typescript` also pulls in `tree-sitter-javascript` (shared grammar for JS/JSX/TSX).

Kotlin is blocked by a tree-sitter version constraint (`>=0.21, <0.23`).

Primary languages (with framework support): TypeScript, Rust, PHP, Dart, Swift, GraphQL, HCL, Bash.
Secondary languages (extraction only): Python, Go, Java, C, C++, C#, Ruby.

## Language Configuration System

Extraction is config-driven via `LanguageConfig`, not per-language extractor implementations. Each language defines:
- `node_mappings`: tree-sitter AST node type -> `NodeKind` (with optional child types for containers)
- `decorator_node_type`: e.g., `"decorator"` for TS, `"attribute_item"` for Rust
- `import_node_types`: which AST nodes represent imports
- `call_node_type` / `call_function_field`: how to find call expressions
- `inheritance_node_types`: AST nodes for extends/implements clauses + their `EdgeKind`
- `export_indicators`: AST nodes/keywords indicating public visibility

Add new language support by: (1) adding tree-sitter grammar dep + feature flag, (2) adding a `LanguageConfig` in `languages/mod.rs`, (3) wiring it in `get_language_config`.

## Gotchas and Non-Obvious Patterns

- **`TreeSitterExtractor` is `&mut self`** because tree-sitter parsers are stateful. `ExtractorRegistry` wraps it in a `Mutex` for thread safety.
- **Duplicate `generate_node_id`**: defined in both `extractor.rs` and `tree_sitter_extractor.rs` with identical logic. The public re-export is from `extractor.rs`.
- **Calls skip nested functions**: `find_calls_recursive` returns early when it encounters a nested function/method definition at depth > 0, so calls inside lambdas/closures declared within a function are NOT attributed to the outer function.
- **`seen_ids` deduplication**: prevents the same node from being extracted twice if it appears via multiple AST paths. If extraction silently drops a node, check if it has a duplicate ID.
- **Go export detection**: uses capitalization convention (`Name` = exported), not AST keywords.
- **Python private detection**: `_name` is private, but `__dunder__` is not treated as private.
- **Snippet lines are 1-indexed**: matching `Node.start_line`/`Node.end_line`. The snippet functions handle the conversion internally.
- **`error.rs` vs `errors.rs`**: `error.rs` defines `ExtractionError` (the main error type). `errors.rs` defines `extract_thrown_errors` (error type extraction from code). Both are public modules.
- **Test detection is regex-based**, not AST-based. `find_test_names_for_symbol` uses pattern matching on source text, not parsed trees.
- **Package name extraction uses simple string parsing** for Cargo.toml, pyproject.toml, go.mod (not a full TOML/YAML parser). This can break on unusual formatting.
- **Scanner skips hidden dirs** except `.codegraph`. Files over `max_file_size` (default 1MB) are silently skipped.
- **Import name extraction** varies by language. For TS/JS it extracts the `source` field (the module path), not individual imported names.

## Test Structure

All tests are in-module (`#[cfg(test)] mod tests`) -- there is no `tests/` directory for integration tests.

- `tree_sitter_extractor.rs` -- per-language extraction tests (gated by `#[cfg(feature = "lang-*")]`): decorators, inheritance, call extraction, exports
- `extractor.rs` -- `ExtractorRegistry` tests, `generate_node_id` determinism
- `orchestrator.rs` -- full pipeline tests with `TempDir`: multi-file indexing, extract-from-source, multi-language
- `scanner.rs` -- file discovery, exclusions (node_modules, hidden dirs), language detection, hash stability
- `snippet.rs` -- truncation, edge cases (empty source, invalid range, exact limit, indentation preservation)
- `test_detection.rs` -- `is_test_file` for all languages, `find_test_names_for_symbol`, import extraction, hybrid test association
- `package.rs` -- manifest parsing for all package types, monorepo/workspace detection, priority ordering, nearest-manifest walk
- `errors.rs` -- thrown error extraction for TS, Rust, Python, Java, Go, Swift, PHP, Ruby, deduplication

Run crate tests:
```bash
cargo test --message-format=json -p codegraph-extraction
```

Run a single language's extractor tests:
```bash
cargo test --message-format=json -p codegraph-extraction --features lang-typescript test_extract_typescript
```
