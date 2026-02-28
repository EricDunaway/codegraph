# CodeGraph MCP Feature Gaps

Features we don't have but should, based on usage patterns and competitive analysis.

## Discovery/Browsing Tools (P0)

The MCP assumes you already know what to search for. Need tools for exploring unfamiliar codebases.

| Gap | Description | Why Needed |
|-----|-------------|------------|
| `codegraph_list_files` | List files in directory or by glob pattern | Can't browse project structure |
| `codegraph_list_directories` | Show project structure tree | Can't see what's in a codebase |
| `codegraph_overview` | Summary stats + top-level structure | Can't answer "what is this codebase?" |

**Ideal exploration workflow (currently impossible):**
1. List directories at root (discovery)
2. List files in target directory (drill down)
3. Get nodes for specific files (inspect)
4. Use callers/callees/impact for deep analysis

## Understanding Tools (P1)

| Gap | Description |
|-----|-------------|
| `analyze_codebase` | Complete project analysis with structure, metrics, complexity |
| `complexity_analysis` | Code complexity with refactoring recommendations |
| `dependency_analysis` | Module/file-level dependency graph (not just node-level) |
| `project_statistics` | Comprehensive health metrics (ours is basic via `codegraph_status`) |

## Query Improvements (P1)

| Gap | Description |
|-----|-------------|
| Natural language query | "What functions handle authentication?" vs keyword "auth" |
| `get_usage_guide` | Self-documenting tool with workflows and examples |

---

## Core Insight

CodeGraph's goal is **exploration and understanding**, not just querying. Tools should help answer:
1. "What is this codebase?" (overview)
2. "Where do I start?" (entry points)
3. "How is it organized?" (structure)
4. "What does X do?" (context + code) — **partially addressed** by `codegraph_context` and `codegraph_node`

Current tools skip 1-3 and assume you're at step 4.

---

*Last updated: 2026-02-28*
