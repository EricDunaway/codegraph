# CodeGraph TypeScript Codebase: Fix Plans

> Comprehensive audit of `/home/user/codegraph/src/` (47 .ts files)
> Generated: 2026-02-06

---

## Priority Matrix

| Priority | Issue | Effort | Risk | Category |
|----------|-------|--------|------|----------|
| **P0 - Critical** | | | | |
| | [1. `any` Types Pervasive in DB Layer](#1-any-types-pervasive-in-db-layer) | Medium | Low | Type Safety |
| | [2. Unguarded `process.exit()` in MCP Server](#2-unguarded-processexit-in-mcp-server) | Low | Medium | Resource Management |
| | [3. SQL Injection via String Interpolation in VSS Search](#3-sql-injection-via-string-interpolation-in-vss-search) | Low | High | Security |
| | [4. Race Condition: Concurrent `initialize()` Calls](#4-race-condition-concurrent-initialize-calls) | Medium | Medium | Concurrency |
| **P1 - High** | | | | |
| | [5. Silent Error Swallowing in Extraction Orchestrator](#5-silent-error-swallowing-in-extraction-orchestrator) | Medium | Low | Error Handling |
| | [6. Unbounded Memory in Brute-Force Vector Search](#6-unbounded-memory-in-brute-force-vector-search) | Medium | Medium | Performance |
| | [7. File URI Parsing Vulnerability in MCP Server](#7-file-uri-parsing-vulnerability-in-mcp-server) | Low | High | Security |
| | [8. No Path Traversal Protection in Import Resolver](#8-no-path-traversal-protection-in-import-resolver) | Medium | High | Security |
| | [9. DFS Stack Overflow on Deep Graphs](#9-dfs-stack-overflow-on-deep-graphs) | Medium | Medium | Performance |
| **P2 - Medium** | | | | |
| | [10. No Database Connection Lifecycle Management](#10-no-database-connection-lifecycle-management) | High | Medium | Resource Management |
| | [11. Framework Detection Reads Every File](#11-framework-detection-reads-every-file) | Medium | Low | Performance |
| | [12. Duplicated ANSI Color Helpers Across Modules](#12-duplicated-ansi-color-helpers-across-modules) | Low | Low | Architecture |
| | [13. `FeatureExtractionPipeline` typed as `any`](#13-featureextractionpipeline-typed-as-any) | Low | Low | Type Safety |
| | [14. Git Hook Script Injection Risk](#14-git-hook-script-injection-risk) | Low | Medium | Security |
| | [15. Missing Input Validation on MCP Tool Arguments](#15-missing-input-validation-on-mcp-tool-arguments) | Medium | Medium | Security |
| | [16. Global Mutable State in Framework Registry](#16-global-mutable-state-in-framework-registry) | Medium | Low | Architecture |
| **P3 - Low** | | | | |
| | [17. findByQualifiedName Scans All Nodes](#17-findbyqualifiedname-scans-all-nodes) | Medium | Low | Performance |
| | [18. Circular Dependency Detection Copies Path Arrays](#18-circular-dependency-detection-copies-path-arrays) | Low | Low | Performance |
| | [19. Hardcoded Import Aliases in Import Resolver](#19-hardcoded-import-aliases-in-import-resolver) | Medium | Low | API Design |
| | [20. Incomplete Node Builtins List in isExternalImport](#20-incomplete-node-builtins-list-in-isexternalimport) | Low | Low | Correctness |
| | [21. `toFloat32Array` Silently Creates Empty Array](#21-tofloat32array-silently-creates-empty-array) | Low | Low | Error Handling |
| | [22. MCP Server Hardcoded Version String](#22-mcp-server-hardcoded-version-string) | Low | Low | API Design |
| | [23. Missing Test Coverage for Framework Resolvers](#23-missing-test-coverage-for-framework-resolvers) | High | Low | Testing |
| | [24. Fuzzy Match Loads All Nodes Into Memory](#24-fuzzy-match-loads-all-nodes-into-memory) | Medium | Low | Performance |

---

## Detailed Fix Plans

---

### 1. `any` Types Pervasive in DB Layer

**Location**: `src/db/queries.ts` (multiple methods), `src/installer/config-writer.ts:51,66`
**Problem Statement**: The `QueryBuilder` class casts all SQLite query results as `any` or uses inline type assertions (`as { count: number }`). The `config-writer.ts` uses `Record<string, any>` throughout. This defeats TypeScript's type system, masks potential runtime errors from schema changes, and makes refactoring dangerous.

**Option A: Define Row Type Interfaces**
- Define a TypeScript interface for every query result shape (e.g., `NodeRow`, `EdgeRow`, `FileRow`, `CountResult`)
- Use generic parameter on `better-sqlite3` `.get<T>()` and `.all<T>()` methods
- Create a mapping layer that transforms raw rows to domain types
- Pros: Full compile-time type safety; catches schema-domain mismatches; IDE autocomplete
- Cons: Verbose; needs updating when schema changes
- Effort: Medium
- Risk: Low

**Option B: Zod Runtime Validation**
- Use `zod` schemas to validate query results at runtime
- Define schemas that mirror expected row shapes
- Parse results through schemas before returning
- Pros: Runtime safety; self-documenting; catches actual data corruption
- Cons: Runtime overhead; additional dependency; more code
- Effort: High
- Risk: Low

**Recommended Option**: Option A
**Tradeoff Analysis**: Option A provides the best bang-for-buck. The codebase already uses TypeScript extensively, so adding row type interfaces is a natural extension. Runtime validation (Option B) is overkill for a local SQLite database where the application controls both reads and writes. The key risk with Option A is staleness if the schema changes, but that risk is mitigated by the fact that the schema is also in this same codebase.

---

### 2. Unguarded `process.exit()` in MCP Server

**Location**: `src/mcp/index.ts:98` (`stop()` method)
**Problem Statement**: `MCPServer.stop()` calls `process.exit(0)` directly. This means if the server is used as a library (not a standalone process), it will kill the host process. It also prevents graceful cleanup of the database connection, vector manager, and any in-flight operations. The `close()` call on line 94 is synchronous but there is no `await` or finalization guarantee before `process.exit`.

**Option A: Remove `process.exit` and Use Cleanup Callbacks**
- Remove `process.exit(0)` from `stop()`
- Add an `onStop` callback or event emitter pattern
- Let the caller decide when/if to exit the process
- Register signal handlers only if the server is the main entry point (e.g., in `bin/codegraph.ts`)
- Pros: Library-safe; proper resource cleanup; testable
- Cons: Breaking change for any code that relies on auto-exit behavior
- Effort: Low
- Risk: Medium

**Option B: Async Shutdown with Timeout**
- Make `stop()` async, await all cleanup
- Add a configurable shutdown timeout
- Only call `process.exit` as a last resort after timeout
- Pros: Guaranteed cleanup; resilient to hanging operations
- Cons: More complex; still calls `process.exit` eventually
- Effort: Medium
- Risk: Low

**Recommended Option**: Option A
**Tradeoff Analysis**: The MCP server should not own process lifecycle. Signal handlers should be registered at the CLI entry point, not inside the library. This separation makes the server embeddable in Electron or other host processes. Option B is a good compromise if backward compatibility is critical.

---

### 3. SQL Injection via String Interpolation in VSS Search

**Location**: `src/vectors/search.ts:333`
**Problem Statement**: The VSS search query uses template literal interpolation for the LIMIT clause: `` LIMIT ${safeLimit} ``. While `safeLimit` is sanitized with `Math.max(1, Math.floor(limit))`, this pattern sets a dangerous precedent. The comment on line 323 acknowledges that "sqlite-vss requires LIMIT to be a literal, not a parameter", but this constraint should be validated and documented more robustly.

**Option A: Validate and Document the Constraint**
- Add explicit bounds checking: `if (limit < 1 || limit > 10000) throw`
- Add a comment with a link to the sqlite-vss issue explaining why parameterized LIMIT is not supported
- Add a unit test that verifies the sanitization works
- Pros: Minimal change; addresses the immediate risk
- Cons: Still uses string interpolation; fragile if copy-pasted
- Effort: Low
- Risk: Low

**Option B: Pre-Generate Prepared Statements with Known Limits**
- Create a small set of prepared statements for common limit values (10, 20, 50, 100, 500)
- Select the nearest ceiling and truncate results in JavaScript
- Pros: No string interpolation at all; zero injection risk
- Cons: Inflexible; wastes resources on larger queries
- Effort: Medium
- Risk: Low

**Recommended Option**: Option A
**Tradeoff Analysis**: The current sanitization (`Math.max(1, Math.floor(limit))`) is actually sufficient to prevent injection since the result is always a positive integer. Option A makes the safety argument explicit and testable. Option B is over-engineered for this specific case where the interpolated value is provably numeric.

---

### 4. Race Condition: Concurrent `initialize()` Calls

**Location**: `src/vectors/manager.ts:87-99`, `src/vectors/embedder.ts:105-145`, `src/index.ts` (multiple `init`/`open` paths)
**Problem Statement**: Both `VectorManager.initialize()` and `TextEmbedder.initialize()` use a simple boolean `initialized` flag to guard against re-initialization. If two callers invoke `initialize()` concurrently (e.g., from parallel MCP tool calls), both will see `initialized === false`, proceed to load the model, and potentially corrupt shared state. The `transformersModule` global variable in `embedder.ts:17` is particularly dangerous -- two concurrent `import()` calls could interleave.

**Option A: Promise-Based Initialization Guard**
- Store the initialization promise instead of a boolean
- On first call, create the promise and store it; on subsequent calls, return the stored promise
- Pattern: `this.initPromise ??= this.doInitialize()`
- Pros: Correct concurrent behavior; no wasted work; simple pattern
- Cons: Requires careful error handling (if init fails, the promise should be reset)
- Effort: Medium
- Risk: Low

**Option B: Mutex/Lock Pattern**
- Use an async mutex (e.g., `async-mutex` npm package or a hand-rolled semaphore)
- Wrap the entire initialization in a critical section
- Pros: General-purpose solution; protects any critical section
- Cons: External dependency; heavier pattern for a one-shot operation
- Effort: Medium
- Risk: Low

**Recommended Option**: Option A
**Tradeoff Analysis**: The promise-based guard is the idiomatic TypeScript/JavaScript pattern for exactly this scenario. It is simpler than a full mutex and handles the common case (multiple callers awaiting the same initialization) perfectly. The mutex pattern is useful for repeated critical sections, but initialization is a one-shot operation.

---

### 5. Silent Error Swallowing in Extraction Orchestrator

**Location**: `src/extraction/index.ts` (file processing loop), `src/resolution/frameworks/index.ts:68-71`
**Problem Statement**: During file extraction, parse errors for individual files are caught and added to an `errors` array, but the orchestrator continues processing. This is correct behavior, but the problem is that the error information is minimal -- just the error message, not the file path, line number, or partial results. Additionally, the framework detection in `detectFrameworks()` catches and swallows all exceptions with an empty `catch {}` block, making it impossible to debug framework detection failures.

**Option A: Structured Error Reporting**
- Define an `ExtractionError` type with `filePath`, `phase`, `message`, `cause` fields
- Replace bare `catch(e)` with structured error construction
- Log framework detection failures at debug level instead of swallowing them
- Add an `onError` callback to `ExtractionOptions` for real-time error reporting
- Pros: Debuggable; actionable error messages; progressive enhancement
- Cons: Some refactoring needed; error types add surface area
- Effort: Medium
- Risk: Low

**Option B: Strict Mode Option**
- Add a `strict: boolean` option that throws on any error instead of accumulating
- Keep the lenient default behavior but give users the option to fail fast
- Pros: Backward compatible; useful for CI/CD pipelines
- Cons: Doesn't improve error quality in lenient mode
- Effort: Low
- Risk: Low

**Recommended Option**: Option A
**Tradeoff Analysis**: Option A addresses the root cause (poor error information) while Option B only changes error *handling* behavior. Structured errors benefit both modes. The framework detection swallowing is particularly concerning because it makes debugging "why isn't my framework detected?" almost impossible.

---

### 6. Unbounded Memory in Brute-Force Vector Search

**Location**: `src/vectors/search.ts:358-387`
**Problem Statement**: The `searchBruteForce` method loads ALL vectors from the database into memory (`SELECT node_id, embedding FROM vectors`), converts each to `Float32Array`, computes cosine similarity, and sorts. For a large codebase with 100K+ nodes, this could use gigabytes of memory (768 dimensions * 4 bytes * 100K = ~300MB just for the embeddings). This is the default path since sqlite-vss is optional.

**Option A: Streaming/Chunked Search**
- Use SQLite's `.iterate()` (supported by `better-sqlite3`) to stream rows
- Maintain a bounded priority queue of top-K results
- Process one row at a time, never holding all embeddings in memory
- Pros: O(K) memory instead of O(N); works with any dataset size
- Cons: Slightly slower due to per-row processing; more complex code
- Effort: Medium
- Risk: Low

**Option B: In-Database Distance Calculation**
- Store embeddings as SQLite BLOBs and compute distance in a custom SQL function registered via `better-sqlite3`'s `db.function()` API
- Return only the top-K results from SQL
- Pros: Leverages SQLite's query optimizer; minimal memory
- Cons: Custom SQL function complexity; floating-point precision concerns
- Effort: High
- Risk: Medium

**Option C: Partition and Cap**
- Add a hard cap on brute-force search (e.g., 10K vectors)
- If the dataset exceeds the cap, warn the user to install sqlite-vss
- Pros: Simple; prevents OOM
- Cons: Degrades functionality; user-unfriendly
- Effort: Low
- Risk: Low

**Recommended Option**: Option A
**Tradeoff Analysis**: Streaming search is the right balance. It works for any dataset size, the memory profile is predictable (O(K) where K is the result limit), and `better-sqlite3`'s `.iterate()` is well-tested. Option B is theoretically superior but the implementation complexity is not justified when sqlite-vss already provides the optimized path. Option C is a stopgap, not a solution.

---

### 7. File URI Parsing Vulnerability in MCP Server

**Location**: `src/mcp/index.ts:163-165`
**Problem Statement**: The MCP server extracts project paths from `rootUri` by doing a simple string replace: `params.rootUri.replace(/^file:\/\//, '')`. This is incorrect for several reasons: (1) It doesn't handle URL-encoded characters (e.g., spaces as `%20`). (2) On Windows, `file:///C:/path` would become `/C:/path` with a leading slash. (3) It doesn't validate that the path actually exists or is within expected bounds. This could lead to the server operating on an unexpected directory.

**Option A: Use Node's `url.fileURLToPath()`**
- Replace the regex with `import { fileURLToPath } from 'url'; fileURLToPath(params.rootUri)`
- Add validation that the resulting path exists and is a directory
- Add validation that it doesn't point to sensitive system directories
- Pros: Correct cross-platform behavior; handles encoded characters; standard API
- Cons: None significant
- Effort: Low
- Risk: Low

**Option B: Use `new URL()` Constructor**
- Parse with `new URL(params.rootUri)` and extract `.pathname`
- Apply platform-specific corrections
- Pros: More control over parsing
- Cons: More code than Option A for the same result
- Effort: Low
- Risk: Low

**Recommended Option**: Option A
**Tradeoff Analysis**: `url.fileURLToPath()` exists specifically for this purpose and handles all the edge cases correctly. There is no reason to hand-roll the conversion. This is a clear bug that should be fixed immediately.

---

### 8. No Path Traversal Protection in Import Resolver

**Location**: `src/resolution/import-resolver.ts:97-124`
**Problem Statement**: The `resolveRelativeImport` function uses `path.resolve(fromDir, importPath)` to resolve relative imports, then calls `context.fileExists()` on the result. If `importPath` contains `../../../etc/passwd` or similar traversal sequences, `path.resolve` will happily resolve it to a path outside the project root. While `context.fileExists()` is limited to the project's indexed files, the `resolveAliasedImport` function's `context.fileExists()` checks could also be bypassed depending on the implementation.

**Option A: Validate Resolved Path Is Within Project Root**
- After resolving, check that the normalized path starts with the project root
- `if (!resolvedAbsPath.startsWith(projectRoot + path.sep)) return null;`
- Pros: Simple; definitive; prevents any traversal attack
- Cons: May reject valid symlinked paths outside the project
- Effort: Low
- Risk: Low

**Option B: Sanitize Input Before Resolution**
- Strip leading `../` sequences and null bytes from import paths
- Reject paths containing suspicious patterns
- Pros: Defense in depth
- Cons: Incomplete (there are many ways to construct traversal paths); blocklist approach is fragile
- Effort: Medium
- Risk: Medium

**Recommended Option**: Option A
**Tradeoff Analysis**: Checking the resolved path is the correct approach because it validates the *result* rather than trying to anticipate all malicious *inputs*. Option B is a defense-in-depth addition but should not be the primary protection.

---

### 9. DFS Stack Overflow on Deep Graphs

**Location**: `src/graph/traversal.ts:154-195`
**Problem Statement**: The `dfsRecursive` method uses actual recursion. For deeply nested codebases (e.g., a function call chain 1000+ levels deep, or a pathological containment hierarchy), this will overflow the JavaScript call stack. The `maxDepth` parameter mitigates this when set to a reasonable value, but the default is `Infinity` (line 14), meaning traversals without explicit depth limits can crash.

**Option A: Convert to Iterative DFS**
- Replace recursive DFS with an explicit stack (array)
- Maintain the same semantics (pre-order traversal, visited set)
- Pros: No stack overflow regardless of depth; same algorithmic complexity
- Cons: Slightly less readable than recursive version
- Effort: Medium
- Risk: Low

**Option B: Set a Reasonable Default `maxDepth`**
- Change `maxDepth` default from `Infinity` to a reasonable value like 100
- Keep the recursive implementation
- Document the limit
- Pros: Minimal change; addresses the practical risk
- Cons: Arbitrary limit; still has a theoretical stack overflow risk
- Effort: Low
- Risk: Low

**Recommended Option**: Option A
**Tradeoff Analysis**: Iterative DFS is a standard algorithm that eliminates the entire class of stack overflow bugs. Since the BFS implementation (lines 48-117) is already iterative (using a queue), making DFS iterative maintains consistency. Option B is a quick fix but leaves a latent bug.

---

### 10. No Database Connection Lifecycle Management

**Location**: `src/db/index.ts`, `src/index.ts` (CodeGraph class)
**Problem Statement**: The `DatabaseConnection` class creates a `better-sqlite3` database connection but has no mechanism to detect or recover from a closed connection. If `.close()` is called and then a query is attempted, `better-sqlite3` will throw a confusing "database is not open" error. Additionally, the `CodeGraph` class exposes both `close()` and `destroy()` methods (lines in `src/index.ts`) but there is no guard preventing use-after-close. The MCP server calls `this.cg.close()` in its `stop()` method but could still receive tool calls after initiating shutdown.

**Option A: State Machine with Connection Guards**
- Add a `state` field to `DatabaseConnection`: `'open' | 'closed' | 'error'`
- Every query method checks state first: `if (this.state !== 'open') throw new DatabaseClosedError()`
- Add `isClosed()` accessor
- Wrap `CodeGraph` methods similarly
- Pros: Clear error messages; prevents use-after-close bugs; debuggable
- Cons: Small overhead per query; adds boilerplate
- Effort: Medium
- Risk: Low

**Option B: Proxy Pattern for Auto-Reconnect**
- Wrap the database with a proxy that auto-reopens on access
- Track the database file path for reconnection
- Pros: Transparent to callers; resilient
- Cons: Hides bugs; state loss on reconnect (e.g., prepared statements invalidated); complex
- Effort: High
- Risk: High

**Recommended Option**: Option A
**Tradeoff Analysis**: Use-after-close is a programmer error, not a recoverable condition. The correct response is a clear exception, not silent reconnection. Option A makes the error obvious at the point of use rather than producing a confusing `better-sqlite3` internal error.

---

### 11. Framework Detection Reads Every File

**Location**: `src/resolution/frameworks/express.ts:29-43`, `src/resolution/frameworks/java.ts:32-47`, `src/resolution/frameworks/swift.ts:15-33`, and similar patterns in most framework resolvers
**Problem Statement**: Many framework `detect()` methods iterate over ALL project files and read their contents. For example, `expressResolver.detect()` reads every file matching 'routes', 'controllers', or 'middleware' directories looking for Express patterns. The `springResolver.detect()` reads every `.java` file looking for annotations. In a 10K-file project, this means potentially reading thousands of files during the detection phase, which runs for EVERY framework resolver (13 resolvers registered).

**Option A: Two-Phase Detection with Fast Rejection**
- Phase 1: Check only marker files (package.json, Cargo.toml, go.mod, etc.) -- these are already cached by the resolution context
- Phase 2: Only if Phase 1 is ambiguous, scan a limited number of files (e.g., first 10 matching files)
- Short-circuit: If Phase 1 definitively detects a framework, skip Phase 2
- Pros: Orders of magnitude faster for most projects; preserves accuracy
- Cons: Some refactoring needed per resolver
- Effort: Medium
- Risk: Low

**Option B: Cache Detection Results**
- Cache the `detect()` result per project/session
- Invalidate on file changes during sync
- Pros: One-time cost; simple
- Cons: Still slow on first run; stale cache risk
- Effort: Low
- Risk: Low

**Recommended Option**: Option A combined with Option B
**Tradeoff Analysis**: Most frameworks can be detected by their package manager manifest (package.json, Cargo.toml, etc.) without scanning source files. Phase 1 alone would eliminate 90%+ of the file reads. Adding caching on top ensures the remaining cost is paid only once. The two approaches are complementary.

---

### 12. Duplicated ANSI Color Helpers Across Modules

**Location**: `src/bin/codegraph.ts:50-74`, `src/installer/banner.ts:13-38`
**Problem Statement**: The ANSI color helper object (`colors` and `chalk`) is duplicated verbatim in both the CLI entry point and the installer banner module. The comments in both files say "avoid chalk ESM issues", but the duplication means any change (e.g., adding a new color) must be made in two places.

**Option A: Extract to Shared Utility**
- Create `src/utils/colors.ts` (or add to existing `src/utils.ts`)
- Export the `chalk` helper object
- Import in both `codegraph.ts` and `banner.ts`
- Pros: DRY; single point of change; no behavior change
- Cons: One more module
- Effort: Low
- Risk: Low

**Option B: Use a Lightweight Color Library**
- Use `picocolors` or `colorette` (tiny, ESM-compatible)
- Remove the hand-rolled color helpers
- Pros: Well-tested; future-proof; smaller code
- Cons: New dependency; must verify ESM/CJS compatibility
- Effort: Low
- Risk: Low

**Recommended Option**: Option A
**Tradeoff Analysis**: Since the project explicitly avoids external color libraries due to ESM issues, extracting the existing helpers to a shared module respects the original design decision while eliminating duplication. If ESM compatibility improves in the future, Option B becomes viable.

---

### 13. `FeatureExtractionPipeline` typed as `any`

**Location**: `src/vectors/embedder.ts:27`
**Problem Statement**: `type FeatureExtractionPipeline = any;` This means all calls to `this.pipeline()` have no type checking. If the `@xenova/transformers` API changes (parameter names, return type), TypeScript will not catch the breakage.

**Option A: Define a Minimal Interface**
- Define a callable interface: `interface Pipeline { (text: string | string[], options: { pooling: string; normalize: boolean }): Promise<{ data: unknown; dims: number[] }> }`
- Use this instead of `any`
- Pros: Type-safe pipeline calls; catches API changes; documents the expected shape
- Cons: May drift from actual API if `@xenova/transformers` changes
- Effort: Low
- Risk: Low

**Option B: Use `typeof import` with Conditional Types**
- Use `Awaited<ReturnType<typeof import('@xenova/transformers')['pipeline']>>` to derive the type
- Pros: Always in sync with the actual library
- Cons: Complex type expressions; may not work well with dynamic imports
- Effort: Medium
- Risk: Medium

**Recommended Option**: Option A
**Tradeoff Analysis**: A minimal interface is pragmatic and maintainable. The pipeline API is stable (it's the core API of the library) and unlikely to change in breaking ways. Option B is theoretically correct but adds complexity and fragility to the type inference chain.

---

### 14. Git Hook Script Injection Risk

**Location**: `src/sync/git-hooks.ts:28-51`
**Problem Statement**: The `POST_COMMIT_SCRIPT` is a hardcoded shell script. This is safe because it does not interpolate any user input. However, the script runs `codegraph sync --quiet` without a `--project` flag, relying on the current working directory. If a user has a `.codegraph/` directory at a parent path, the hook could sync the wrong project. Additionally, the script uses `npx codegraph` as a fallback, which could execute an attacker-controlled package if the project has a malicious `.npmrc` or if a typosquat package exists.

**Option A: Pin the Project Path in the Hook Script**
- Include the absolute project path in the generated hook script
- `codegraph sync --quiet --path "/absolute/path/to/project"`
- Validate that the path still exists before running
- Pros: Deterministic; immune to CWD changes
- Cons: Hook breaks if project is moved
- Effort: Low
- Risk: Low

**Option B: Validate npx Target**
- Replace `npx codegraph` with `npx @colbymchenry/codegraph` to use the scoped package name
- Add a hash verification step
- Pros: Prevents typosquatting
- Cons: Scoped name is longer; doesn't address the CWD issue
- Effort: Low
- Risk: Low

**Recommended Option**: Both Options A and B
**Tradeoff Analysis**: These are independent fixes addressing different risks. Pinning the project path (Option A) prevents wrong-project syncing. Using the scoped package name (Option B) prevents typosquatting. Both are low-effort and should be applied together.

---

### 15. Missing Input Validation on MCP Tool Arguments

**Location**: `src/mcp/tools.ts:219-388`
**Problem Statement**: MCP tool handlers cast arguments directly with `args.query as string`, `args.symbol as string`, etc., without validating that the values are actually strings. If a client sends `{ "query": 42 }` or `{ "query": null }`, the tool will pass a non-string value downstream, potentially causing confusing errors deep in the call stack. The `limit` parameter is cast with `(args.limit as number) || 10` which is slightly better but still doesn't validate type.

**Option A: Add Validation Functions**
- Create a `validateArgs` helper that checks required fields and types
- Return `errorResult` early if validation fails
- Example: `if (typeof args.query !== 'string') return this.errorResult('query must be a string')`
- Pros: Clear error messages; prevents downstream failures; simple
- Cons: Boilerplate per tool; needs maintenance when tools change
- Effort: Medium
- Risk: Low

**Option B: Use Zod Schemas for Tool Arguments**
- Define a zod schema per tool matching the `inputSchema`
- Parse `args` through the schema before processing
- Pros: Single source of truth for validation; auto-generates error messages; composable
- Cons: New dependency; may be overkill for 7 tools
- Effort: Medium
- Risk: Low

**Recommended Option**: Option A
**Tradeoff Analysis**: With only 7 tools, the validation code is manageable without a schema library. Each tool already has its own handler function, so adding 2-3 lines of validation per handler is straightforward. Option B would be better if the number of tools grows significantly.

---

### 16. Global Mutable State in Framework Registry

**Location**: `src/resolution/frameworks/index.ts:22-46,78-84`
**Problem Statement**: `FRAMEWORK_RESOLVERS` is a module-level `const` array that is mutated by `registerFrameworkResolver()` (push/splice). This is global mutable state that is shared across all `CodeGraph` instances in the same process. If two instances register different custom resolvers, they will interfere with each other. This also makes testing unreliable since test order can affect results.

**Option A: Instance-Level Registry**
- Move the resolver array into the `ReferenceResolver` class (or `CodeGraph` class)
- Pass it as a constructor parameter with the default resolvers
- `registerFrameworkResolver` becomes a method on the instance
- Pros: No shared state; testable; concurrent-safe
- Cons: Breaking change for any code calling the standalone `registerFrameworkResolver`
- Effort: Medium
- Risk: Low

**Option B: Copy-on-Read**
- Change `getAllFrameworkResolvers()` to return a copy of the array
- Store registered resolvers in a separate array and merge on read
- Pros: Backward compatible; reduces interference
- Cons: Does not fully eliminate shared state; wasteful copies
- Effort: Low
- Risk: Low

**Recommended Option**: Option A
**Tradeoff Analysis**: The framework registry should be instance-scoped. Global mutable state is one of the most common sources of subtle bugs in long-running processes (like an MCP server that could handle multiple project roots over its lifetime). Option A is a clean fix. The breaking change is acceptable since `registerFrameworkResolver` is not part of the documented public API.

---

### 17. findByQualifiedName Scans All Nodes

**Location**: `src/graph/queries.ts:184-216`
**Problem Statement**: `findByQualifiedName` iterates over all nodes of each kind, testing a regex against `qualifiedName`. The comment on line 193 acknowledges this: "This is inefficient for large graphs - would need FTS index on qualified_name". For a codebase with 100K+ nodes, this is O(N) per query.

**Option A: Add FTS5 Index on qualified_name**
- Add `qualified_name` to the existing FTS5 table or create a new one
- Use FTS5 prefix queries for glob-like matching
- Pros: O(log N) lookups; leverages existing SQLite infrastructure
- Cons: Increases database size; FTS5 may not support arbitrary regex patterns
- Effort: Medium
- Risk: Low

**Option B: Use SQL LIKE or GLOB**
- Translate the pattern to a SQL LIKE clause
- Add a B-tree index on `qualified_name`
- Pros: Simple; standard SQL; index helps with prefix queries
- Cons: Limited pattern support (no arbitrary regex); LIKE is still O(N) without prefix
- Effort: Low
- Risk: Low

**Recommended Option**: Option B as a quick win, Option A as a follow-up
**Tradeoff Analysis**: Most qualified name searches use prefix patterns (e.g., `src/auth/*`), which benefit from a B-tree index with LIKE. FTS5 is more powerful but may not be needed. Start with Option B and upgrade to Option A if users report performance issues.

---

### 18. Circular Dependency Detection Copies Path Arrays

**Location**: `src/graph/queries.ts:271` (`[...path, filePath]`)
**Problem Statement**: The `findCircularDependencies` DFS creates a new array copy at every recursion step: `dfs(dep, [...path, filePath])`. For a file dependency graph with V vertices and E edges, this is O(V * E) memory in the worst case. A standard cycle detection algorithm (Tarjan's or colored DFS) uses O(V) memory.

**Option A: Use Colored DFS (White/Gray/Black)**
- Replace the path-copying DFS with a standard three-color cycle detection
- Use the `recursionStack` set (already present) as the "gray" set
- Reconstruct cycles from the stack when found
- Pros: O(V) memory; O(V + E) time; standard algorithm
- Cons: Cycle reconstruction is slightly more complex
- Effort: Low
- Risk: Low

**Option B: Limit Detection Scope**
- Only detect cycles among files with recent changes
- Cap the number of files analyzed
- Pros: Fast in practice
- Cons: May miss cycles; changes semantics
- Effort: Low
- Risk: Medium

**Recommended Option**: Option A
**Tradeoff Analysis**: The colored DFS is strictly better -- same semantics, better complexity, and it is a well-known algorithm. The existing code already has `visited` and `recursionStack` sets, which are essentially the white and gray sets. The only missing piece is reconstructing the cycle from the stack instead of the path array.

---

### 19. Hardcoded Import Aliases in Import Resolver

**Location**: `src/resolution/import-resolver.ts:138-145`
**Problem Statement**: The `resolveAliasedImport` function contains a hardcoded map of path aliases (`@/ -> src/`, `~/ -> src/`, etc.). Real projects configure aliases in `tsconfig.json` (`paths`), `webpack.config.js` (`resolve.alias`), or `vite.config.ts`. The hardcoded map will miss project-specific aliases and may incorrectly resolve paths in projects that use different conventions.

**Option A: Read tsconfig.json paths**
- Parse `tsconfig.json` to extract `compilerOptions.paths`
- Transform the tsconfig paths entries into the alias map
- Fall back to hardcoded aliases if tsconfig is not found
- Pros: Correct for TypeScript projects; respects project configuration
- Cons: Only handles TypeScript; tsconfig extends chains are complex to resolve
- Effort: Medium
- Risk: Medium

**Option B: Configurable Alias Map**
- Add an `aliases` field to `CodeGraphConfig`
- Let users specify their path aliases in `.codegraph/config.json`
- Fall back to auto-detection (tsconfig) or hardcoded defaults
- Pros: Flexible; language-agnostic; user-controllable
- Cons: Requires user configuration; another config surface area
- Effort: Medium
- Risk: Low

**Recommended Option**: Option A with Option B as an override
**Tradeoff Analysis**: TypeScript projects are the primary audience, and `tsconfig.json` is the canonical source of path aliases for them. Option A provides correct behavior for the majority of users. Option B provides an escape hatch for non-TypeScript projects or unusual configurations.

---

### 20. Incomplete Node Builtins List in isExternalImport

**Location**: `src/resolution/import-resolver.ts:66`
**Problem Statement**: The list of Node.js built-in modules is manually maintained and incomplete. It is missing many builtins: `assert`, `dns`, `net`, `tls`, `zlib`, `worker_threads`, `v8`, `perf_hooks`, `readline`, `querystring`, `string_decoder`, `tty`, `dgram`, and all `node:` prefixed imports. Any import of a missing builtin will be incorrectly treated as a local/aliased import, leading to failed resolution attempts.

**Option A: Use `module.builtinModules`**
- Replace the hardcoded list with `require('module').builtinModules`
- Also check for `node:` prefix: `if (importPath.startsWith('node:')) return true`
- Pros: Always complete; automatically includes new builtins in newer Node.js versions
- Cons: Requires the `module` module (always available in Node.js)
- Effort: Low
- Risk: Low

**Option B: Comprehensive Hardcoded List**
- Expand the existing list to include all known builtins
- Add `node:` prefix handling
- Pros: No dynamic dependency; works in non-Node environments
- Cons: Must be manually updated for new Node.js releases
- Effort: Low
- Risk: Low

**Recommended Option**: Option A
**Tradeoff Analysis**: Since CodeGraph runs on Node.js, `module.builtinModules` is always available and always correct. There is no reason to maintain a manual list when the runtime provides the authoritative one.

---

### 21. `toFloat32Array` Silently Creates Empty Array

**Location**: `src/vectors/embedder.ts:296-299`
**Problem Statement**: The `ArrayLike<number>` branch of `toFloat32Array` creates `new Float32Array(arr.length)` -- a zero-filled array -- instead of copying the data from `arr`. This means that if the ONNX output is an ArrayLike (not a Float32Array and not a plain Array), the embedding will be all zeros, and all similarity scores will be 0. This is a silent data corruption bug.

**Option A: Copy Data Correctly**
- Change to `Float32Array.from(arr)` or `new Float32Array(Array.from(arr))`
- Add a test that verifies non-zero embeddings for known input
- Pros: Correct behavior; minimal change
- Cons: None
- Effort: Low
- Risk: Low

**Option B: Throw on Unrecognized Format**
- Remove the ArrayLike branch entirely
- Let it fall through to the `throw` on line 301
- Pros: Fail-fast; makes unexpected formats visible
- Cons: May break if the ONNX runtime changes its output format
- Effort: Low
- Risk: Low

**Recommended Option**: Option A
**Tradeoff Analysis**: The ArrayLike branch was clearly intended to handle a real case (typed arrays other than Float32Array). The fix is to actually copy the data. Option B is more aggressive and could cause regressions if the ONNX runtime legitimately returns ArrayLike objects.

---

### 22. MCP Server Hardcoded Version String

**Location**: `src/mcp/index.ts:27`
**Problem Statement**: `SERVER_INFO.version` is hardcoded as `'0.1.0'`. The CLI correctly reads the version from `package.json`, but the MCP server does not. This means the version reported to MCP clients will always be `0.1.0` regardless of the actual package version, making debugging version mismatches impossible.

**Option A: Read from package.json**
- Import or read `package.json` at module load time
- Use the `version` field for `SERVER_INFO.version`
- Pros: Always accurate; single source of truth
- Cons: Requires resolving `package.json` path at runtime
- Effort: Low
- Risk: Low

**Option B: Generate version at build time**
- Use a build step to replace the version constant
- Pros: No runtime file reading; works in bundled environments
- Cons: Requires build tooling changes; easy to forget
- Effort: Medium
- Risk: Low

**Recommended Option**: Option A
**Tradeoff Analysis**: The CLI already does this (reading `package.json` at runtime), so the pattern is established. Apply the same approach to the MCP server.

---

### 23. Missing Test Coverage for Framework Resolvers

**Location**: `__tests__/resolution.test.ts` (limited scope), all `src/resolution/frameworks/*.ts`
**Problem Statement**: There are 13 framework resolvers covering React, Express, Laravel, Django, Flask, FastAPI, Rails, Spring, Go, Rust, ASP.NET, SwiftUI, UIKit, and Vapor. Each has `detect()`, `resolve()`, and often `extractNodes()` methods. Based on the test structure described in `CLAUDE.md`, the resolution tests focus on the general resolution pipeline, not individual framework resolvers. This means the framework-specific regex patterns (which are notoriously brittle) are untested.

**Option A: Unit Tests Per Framework**
- Create `__tests__/frameworks/` directory with one test file per framework
- Test each `detect()` with realistic and edge-case file structures
- Test each `resolve()` with known reference patterns
- Test each `extractNodes()` with realistic source code snippets
- Pros: Comprehensive; catches regex bugs; documents expected behavior
- Cons: Many test files; requires creating realistic fixtures for each framework
- Effort: High
- Risk: Low

**Option B: Property-Based Tests**
- Use a property-based testing library (e.g., `fast-check`) to generate random code-like strings
- Verify that `extractNodes` never throws and returns valid `Node` objects
- Pros: Catches edge cases that manual tests miss; compact
- Cons: Less readable; doesn't test specific behavior; may produce false negatives
- Effort: Medium
- Risk: Low

**Recommended Option**: Option A
**Tradeoff Analysis**: Framework resolvers are pattern-matching code that is inherently fragile. The only way to have confidence in them is to test specific patterns against known inputs. Option B is a good supplement but cannot verify correctness (only robustness). Start with the most critical frameworks (React, Express, Spring) and expand coverage over time.

---

### 24. Fuzzy Match Loads All Nodes Into Memory

**Location**: `src/resolution/name-matcher.ts:196-237`
**Problem Statement**: The `matchFuzzy` function loads ALL functions, methods, and classes from the resolution context to perform case-insensitive and prefix matching. For large codebases, this creates a massive array that is linearly scanned twice (once for case-insensitive match, once for prefix match). This function is called for every unresolved reference that fails all higher-confidence resolution strategies.

**Option A: Case-Insensitive Search in SQLite**
- Add a COLLATE NOCASE index on `name` in the nodes table
- Use SQL for case-insensitive matching: `WHERE name = ? COLLATE NOCASE`
- Limit results in SQL instead of loading all into memory
- Pros: O(log N) instead of O(N); no memory spike; leverages database
- Cons: Requires schema change and migration
- Effort: Medium
- Risk: Low

**Option B: Skip Fuzzy Matching by Default**
- Make fuzzy matching opt-in via a configuration flag
- Most users would prefer accurate results over fuzzy guesses with 0.3-0.5 confidence
- Pros: Eliminates the performance issue entirely; reduces false positive resolutions
- Cons: May reduce resolution rate for some projects
- Effort: Low
- Risk: Low

**Recommended Option**: Option A
**Tradeoff Analysis**: Fuzzy matching serves a real purpose (resolving references that differ only in case), but it should be efficient. Moving the search to SQL is the right approach since the database already has all the data. Option B is worth considering as a complementary change -- low-confidence fuzzy matches (0.3) may do more harm than good.

---

## Cross-Cutting Concerns

### Error Handling Strategy Summary

The codebase has an inconsistent error handling strategy:
- **Extraction errors**: Accumulated in an array, processing continues (correct for robustness)
- **Framework detection errors**: Silently swallowed (problematic for debugging)
- **Database errors**: Let `better-sqlite3` throw (inconsistent error types)
- **MCP tool errors**: Caught at the handler level and formatted as `ToolResult` (correct)
- **Vector errors**: Mix of thrown errors and silent fallbacks (inconsistent)

**Recommendation**: Establish a layered error handling policy:
1. **Infrastructure layer** (db, vectors): Throw typed errors (`DatabaseError`, `VectorError`)
2. **Domain layer** (extraction, resolution, graph): Accumulate or throw depending on severity
3. **API layer** (MCP, CLI): Catch, format, and report

### Concurrency Model Summary

The codebase is largely single-threaded (appropriate for Node.js), but has these concurrency risks:
- Concurrent `initialize()` calls (Issue #4)
- MCP server handling concurrent tool calls while shutting down (Issue #2, #10)
- Git hooks running `codegraph sync` in background while user runs CLI commands

**Recommendation**: Use promise-based guards for initialization, connection state guards for use-after-close, and file locking (e.g., `proper-lockfile`) for the SQLite database during sync operations.

### Resource Management Summary

Resources that need lifecycle management:
- SQLite database connections (opened in `CodeGraph.open()`, closed in `close()`/`destroy()`)
- ONNX model (loaded in `TextEmbedder.initialize()`, released in `dispose()`)
- Tree-sitter parsers (implied by `ExtractionOrchestrator`)
- File handles (opened during extraction and code reading)

**Recommendation**: Implement a `Disposable` pattern (TypeScript 5.2+ `using` keyword or a manual `.dispose()` convention) across all resource-holding classes. Add a cleanup check in `CodeGraph.close()` that disposes all sub-managers.

---

## Appendix: Files Audited

All 47 TypeScript files and 1 SQL file were read and analyzed:

**Core**: `index.ts`, `types.ts`, `config.ts`, `errors.ts`, `directory.ts`, `utils.ts`
**Database**: `db/index.ts`, `db/queries.ts`, `db/migrations.ts`, `db/schema.sql`
**Extraction**: `extraction/index.ts`, `extraction/tree-sitter.ts`, `extraction/grammars.ts`
**Resolution**: `resolution/index.ts`, `resolution/types.ts`, `resolution/import-resolver.ts`, `resolution/name-matcher.ts`, `resolution/frameworks/index.ts`, `resolution/frameworks/react.ts`, `resolution/frameworks/express.ts`, `resolution/frameworks/laravel.ts`, `resolution/frameworks/python.ts`, `resolution/frameworks/ruby.ts`, `resolution/frameworks/java.ts`, `resolution/frameworks/go.ts`, `resolution/frameworks/rust.ts`, `resolution/frameworks/csharp.ts`, `resolution/frameworks/swift.ts`
**Graph**: `graph/index.ts`, `graph/traversal.ts`, `graph/queries.ts`
**Vectors**: `vectors/index.ts`, `vectors/manager.ts`, `vectors/embedder.ts`, `vectors/search.ts`
**Context**: `context/index.ts`, `context/formatter.ts`
**Sync**: `sync/index.ts`, `sync/git-hooks.ts`
**MCP**: `mcp/index.ts`, `mcp/tools.ts`, `mcp/transport.ts`
**Installer**: `installer/index.ts`, `installer/banner.ts`, `installer/config-writer.ts`, `installer/prompts.ts`, `installer/claude-md-template.ts`
**CLI**: `bin/codegraph.ts`
