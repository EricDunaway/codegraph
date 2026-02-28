# Documentation Standards

This directory contains project documentation. Each doc type has its own format.

## Plan Documents (`plans/`)

### File naming
`YYYY-MM-DD-<topic>.md` — date is when the plan was created.

### Required header
```markdown
# <Title>

**Date:** YYYY-MM-DD
**Status:** Draft | Active | Completed (IMPLEMENTED) | Completed (SUPERSEDED by `<link>`)
**Crates affected:** `codegraph-foo`, `codegraph-bar`
**Depends on:** `<filename>` (optional)
```

### Status lifecycle
- **Draft** — proposed, not yet approved for implementation
- **Active** — approved, work underway or planned
- **Completed (IMPLEMENTED)** — verified working, move to `plans/completed/`
- **Completed (SUPERSEDED by ...)** — replaced by another plan, move to `plans/completed/`

Update status in the file *before* moving it.

### Recommended sections
Include what's relevant, skip what isn't:
- **Purpose** — what and why (1-2 paragraphs)
- **Non-Goals** — explicit scope boundaries
- **Design Decisions** — key choices with rationale
- **Implementation** — approach, phases, milestones
- **Testing** — how to verify the work
- **Open Questions** — unresolved decisions

### Completion rule
When a plan's tasks are all done:
1. Update `**Status:**` to `Completed (IMPLEMENTED)` with date
2. Move file to `plans/completed/`
3. Update the plan references in root `CLAUDE.md`

## Issues (`issues.md`)

Bugs where capabilities don't match the spec/schema.

### Per-issue format
```markdown
### <description> (P0/P1/P2)

**Schema:** what exists
**Actual:** what's happening
**Impact:** user-facing consequence
**Location:** `crates/<name>/`

(optional SQL verification query)
```

### Rules
- **Remove when fixed** — don't track resolved issues
- **Priority levels:** P0 = blocking/broken, P1 = important but workaround exists, P2 = nice to fix
- Verify claims with actual queries before writing

## Feature Gaps (`gaps.md`)

Missing capabilities that would improve the product.

### Per-gap format
```markdown
### <feature name> (P0/P1/P2)

Brief description of what's missing and why it matters.
```

### Rules
- **Remove when implemented** — don't track completed features
- **Priority levels:** P0 = users ask for this, P1 = would improve experience, P2 = nice to have

## General Rules (all doc types)

- **Date-stamp updates:** end every file with `*Last updated: YYYY-MM-DD*`
- **Verify against code** — never write claims without checking the implementation
- **Link related docs** — plans should reference relevant issues/gaps and vice versa
- **Keep it concise** — document decisions and gotchas, not obvious things
