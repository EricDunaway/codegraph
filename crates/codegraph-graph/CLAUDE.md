# codegraph-graph

Graph traversal algorithms and high-level graph queries for the code knowledge graph.

Dependencies: `codegraph-types`, `codegraph-db`, `rusqlite`, `thiserror`

Source: `crates/codegraph-graph/src/`

## Modules

| File | What it does |
|------|-------------|
| `lib.rs` | Re-exports `GraphTraverser`, `TraversalResult`, `GraphQueryManager`, `CallGraph`, `ImpactRadius`, `CircularDependency`, `DeadCodeResult`, `NodeMetrics` |
| `traversal.rs` | BFS/DFS traversal, callers/callees, path finding, embedding neighbor methods |
| `queries.rs` | Call graph construction, impact radius, circular deps, dead code, node metrics, type hierarchy |
| `error.rs` | `GraphError` enum (`NodeNotFound`, `CycleDetected`, `Database`, `InvalidOptions`, `LimitExceeded`) |

## GraphTraverser (traversal.rs)

Holds `&Connection` + `&mut QueryBuilder`. All methods return `Result<_, GraphError>`.

### Core traversal

```rust
fn bfs(&mut self, start_id: &str, options: &TraversalOptions) -> Result<TraversalResult, GraphError>
fn dfs(&mut self, start_id: &str, options: &TraversalOptions) -> Result<TraversalResult, GraphError>
```

Both use `TraversalOptions` to control direction, edge/node filters, depth, and limit. BFS uses `VecDeque`, DFS uses `Vec` as stack. Start node is included in result only if `options.include_start` is true.

### Caller/callee queries

```rust
fn get_callers(&mut self, node_id: &str) -> Result<Vec<Node>, GraphError>   // incoming Calls edges
fn get_callees(&mut self, node_id: &str) -> Result<Vec<Node>, GraphError>   // outgoing Calls edges
```

### Path finding

```rust
fn find_path(&mut self, from_id: &str, to_id: &str, max_depth: u32) -> Result<Option<Vec<NodeId>>, GraphError>
```

BFS shortest-path over **all** outgoing edge kinds (not just Calls). Returns `None` if no path exists within `max_depth`.

### Embedding context methods (direct-only, cycle-safe)

These return **names** (not IDs), limited by `limit`. Used by `codegraph-vectors` to build enriched text for embeddings.

```rust
fn get_callees_for_embedding(&mut self, node_id: &str, limit: usize) -> Result<Vec<String>, GraphError>
fn get_callers_for_embedding(&mut self, node_id: &str, limit: usize) -> Result<Vec<String>, GraphError>
fn get_siblings_for_embedding(&mut self, node_id: &str, limit: usize) -> Result<Vec<String>, GraphError>
fn get_implements_for_embedding(&mut self, node_id: &str, limit: usize) -> Result<Vec<String>, GraphError>
fn get_extends_for_embedding(&mut self, node_id: &str) -> Result<Option<String>, GraphError>
```

**Sorting priority** (applied to callees/callers): decorated nodes first, then higher incoming-call frequency, then alphabetical.

### Embedding sync methods (1-hop neighbor detection)

Used by incremental sync to find nodes whose embedding text is stale after a change.

```rust
fn get_embedding_neighbors(&mut self, node_ids: &[&str]) -> Result<HashSet<String>, GraphError>
```
Returns IDs of nodes connected by Calls (both directions), Extends (incoming), or Implements (incoming). Excludes the input set.

```rust
fn get_embedding_siblings(&mut self, node_ids: &[&str]) -> Result<HashSet<String>, GraphError>
```
Returns IDs of nodes sharing a Contains parent with any input node. Excludes the input set.

## GraphQueryManager (queries.rs)

Holds `&Connection` + `&mut QueryBuilder`. All methods return `Result<_, GraphError>`.

### build_call_graph

```rust
fn build_call_graph(&mut self, root_ids: &[&str]) -> Result<CallGraph, GraphError>
```

Recursively follows outgoing `Calls` edges from the given root IDs. Populates `CallGraph.entry_points` (no incoming calls) and `CallGraph.leaf_nodes` (no outgoing calls).

### get_impact_radius

```rust
fn get_impact_radius(&mut self, node_id: &str, max_depth: u32) -> Result<ImpactRadius, GraphError>
```

BFS over **incoming** Calls/References/Imports edges. Depth 0 callers go into `direct`, deeper ones into `indirect`.

### find_circular_dependencies

```rust
fn find_circular_dependencies(&mut self) -> Result<Vec<CircularDependency>, GraphError>
```

DFS cycle detection over **File** nodes following **Imports** edges only. Returns cycles as node ID + file path lists.

### find_dead_code

```rust
fn find_dead_code(&mut self) -> Result<DeadCodeResult, GraphError>
```

Finds:
- Functions with no incoming Calls edges (excludes `main` and `test_*` prefixed)
- Classes/Structs/Interfaces with no incoming Instantiates/Extends/Implements/TypeOf/References edges
- Exports with no incoming Imports edges

### get_node_metrics

```rust
fn get_node_metrics(&mut self, node_id: &str) -> Result<NodeMetrics, GraphError>
```

Counts in/out degree, dependencies/dependents (excludes Contains edges), complexity estimate (1 + outgoing Calls count). Does **not** compute `depth` (always `None`).

### get_type_hierarchy

```rust
fn get_type_hierarchy(&mut self, type_id: &str) -> Result<Subgraph, GraphError>
```

Walks Extends/Implements edges both upward (outgoing = parents) and downward (incoming = children) to build a full inheritance tree.

## Key Data Structures

### TraversalResult
```
subgraph: Subgraph       // visited nodes + edges
max_depth_reached: u32   // actual max depth
truncated: bool          // true if limit was hit
```

### CallGraph
```
subgraph: Subgraph           // call relationship graph
entry_points: Vec<NodeId>    // no incoming calls
leaf_nodes: Vec<NodeId>      // no outgoing calls
```

### ImpactRadius
```
direct: Vec<Node>       // depth-0 callers (immediate)
indirect: Vec<Node>     // depth > 0 callers (transitive)
total_count: usize      // direct.len() + indirect.len()
max_depth: u32          // deepest level reached
```

### CircularDependency
```
nodes: Vec<NodeId>      // IDs in the cycle
files: Vec<String>      // file paths involved
```

### DeadCodeResult
```
unused_functions: Vec<Node>   // no incoming Calls
unused_types: Vec<Node>       // no incoming Instantiates/Extends/Implements/TypeOf/References
unused_exports: Vec<Node>     // no incoming Imports
total_count: usize
```

### NodeMetrics
```
in_degree: usize        // all incoming edges
out_degree: usize       // all outgoing edges
dependencies: usize     // outgoing non-Contains edges
dependents: usize       // incoming non-Contains edges
depth: Option<u32>      // always None (not computed)
complexity: usize       // 1 + outgoing Calls count
```

## TraversalOptions (from codegraph-types)

```rust
TraversalOptions {
    direction: TraversalDirection,       // Outgoing (default), Incoming, Both
    edge_kinds: Option<Vec<EdgeKind>>,   // filter edges (None = all)
    node_kinds: Option<Vec<NodeKind>>,   // filter nodes (None = all)
    max_depth: Option<u32>,              // None = unlimited
    limit: Option<usize>,               // max nodes returned; None = unlimited
    include_start: bool,                 // include start node in result
}
```

## Gotchas

- **Both structs borrow `&mut QueryBuilder`**: You cannot hold a `GraphTraverser` and `GraphQueryManager` simultaneously on the same `QueryBuilder`. Create one, use it, drop it, then create the other.
- **`find_path` traverses all edge kinds**: Unlike `get_callers`/`get_callees` which filter to Calls edges, `find_path` follows all outgoing edges. This means a path may go through Contains, Imports, etc.
- **`NodeMetrics.depth` is always `None`**: The field exists but `get_node_metrics` never computes it.
- **`find_dead_code` skips `main` and `test_*`**: Functions named `main` or starting with `test_` are never flagged as dead code. No other heuristics for entry points (e.g., exported functions, framework handlers).
- **`find_circular_dependencies` is file-level only**: It looks at File nodes + Imports edges. It will not detect function-level call cycles.
- **Embedding methods are direct-only (1-hop)**: The `*_for_embedding` methods intentionally skip transitive traversal, making them safe against cycles.
- **`count_incoming_calls` uses raw SQL**: The `sort_nodes_by_priority` helper runs `SELECT COUNT(*) FROM edges WHERE target = ?1 AND kind = 'calls'` directly against the connection, bypassing `QueryBuilder`.
- **`get_impact_radius` depth semantics**: Depth 0 means "direct caller of the input node." The input node itself is never included in the result.
