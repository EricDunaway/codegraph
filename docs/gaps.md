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

| Gap | Description | Competitor Has |
|-----|-------------|----------------|
| `analyze_codebase` | Complete project analysis with structure, metrics, complexity | Other tool |
| `complexity_analysis` | Code complexity with refactoring recommendations | Other tool |
| `dependency_analysis` | Module/file-level dependency graph (not just node-level) | Other tool |
| `project_statistics` | Comprehensive health metrics | Other tool (ours is basic) |

## Query Improvements (P1)

| Gap | Description | Competitor Has |
|-----|-------------|----------------|
| Natural language query | "What functions handle authentication?" vs keyword "auth" | CodeRAG |
| `get_usage_guide` | Self-documenting tool with workflows and examples | Other tool |

## Code Display (P0)

| Gap | Description | Impact |
|-----|-------------|--------|
| `codegraph_node` should return actual code | Currently just returns file:line pointer | User still has to read file manually |

---

## Core Insight

CodeGraph's goal is **exploration and understanding**, not just querying. Tools should help answer:
1. "What is this codebase?" (overview)
2. "Where do I start?" (entry points)
3. "How is it organized?" (structure)
4. "What does X do?" (context + code)

Current tools skip 1-3 and assume you're at step 4.

---

## Competitive Reference

**CodeRAG tools:**
- `list_directory` - file browsing
- `query_code_graph` - natural language queries
- `get_code_snippet` - actual code by qualified name

**Other tool:**
- `analyze_codebase` - complete project analysis
- `complexity_analysis` - code quality
- `dependency_analysis` - module-level deps
- `project_statistics` - health metrics
- `get_usage_guide` - self-documenting

---

*Last updated: 2026-02-05*
