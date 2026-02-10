# Rectifying the Framework Feature Gap in Rust (Idiomatic Design)

Date: 2026-02-10

## Goal

Close the TypeScript-vs-Rust framework resolution gap by introducing a modular, testable, and performant framework-resolution architecture in Rust without sacrificing safety or maintainability.

## Current gap (summary)

- TypeScript uses a registry of framework resolvers (React, Express, Laravel, Django, FastAPI, Rails, etc.).
- Rust currently relies on generic strategies plus a small set of inline heuristics.

## Idiomatic Rust target architecture

## 1) Introduce a `FrameworkResolver` trait

```rust
pub trait FrameworkResolver: Send + Sync {
    fn name(&self) -> &'static str;
    fn detect(&self, ctx: &ResolutionContext) -> bool;
    fn resolve(
        &self,
        ctx: &ResolutionContext,
        source: &Node,
        ref_name: &str,
    ) -> Result<Option<ResolvedTarget>, ResolutionError>;
}
```

Why idiomatic:
- Trait-based polymorphism keeps resolution behavior open for extension.
- `Send + Sync` supports future parallel resolution.
- Small method set preserves clear responsibility boundaries.

## 2) Add a `FrameworkRegistry`

Create `FrameworkRegistry` in `codegraph-resolution`:

- Stores `Vec<Box<dyn FrameworkResolver>>`.
- Supports deterministic ordering (priority based).
- Exposes `detect_active_frameworks(context)` and `resolve_with_active_frameworks(...)`.

Why idiomatic:
- Explicit dependency injection into `ReferenceResolver`.
- Avoids giant `match`/`if` heuristic chains.
- Keeps framework modules independent and unit-testable.

## 3) Define `ResolutionContext` as a typed struct

Use a narrow, borrow-friendly context type:

- project root path
- file path / language
- import graph snapshot
- query interface handles (abstractions over DB lookups)

Why idiomatic:
- Minimizes ad-hoc global access.
- Makes resolver behavior deterministic and easier to test with fixtures.

## 4) Split framework modules per file

Proposed layout:

- `crates/codegraph-resolution/src/frameworks/mod.rs`
- `react.rs`
- `express.rs`
- `django.rs`
- `fastapi.rs`
- `rails.rs`
- `spring.rs`
- etc.

Each module should:
- implement `FrameworkResolver`
- include focused tests for detection + resolution cases
- avoid cross-module coupling

## 5) Integrate with existing resolution pipeline

In `ReferenceResolver::resolve_reference_with_kind(...)` order should become:

1. built-in filtering
2. import resolution
3. exact/qualified/fuzzy name matching
4. framework registry resolution (active frameworks only)

This preserves existing behavior while adding framework-specific wins as a late-stage enhancer.

## 6) Add confidence + provenance contracts

Extend `ResolvedTarget` metadata with:

- resolver name (`"react"`, `"django"`)
- rule ID (`"jsx_component_suffix"`, etc.)
- confidence score buckets (`1.0`, `0.9`, `0.8`)

Why:
- Better observability/debugging.
- Easier safety controls (e.g., only persist edges above threshold).

## 7) Add guardrails for false positives

- Framework resolvers should return `None` on ambiguity unless a tie-breaker exists.
- Prefer same-file / same-module targets.
- Use small rule-based confidence penalties for cross-file weak matches.

## Implementation phases

## Phase 1 (foundation)

- Introduce trait + registry + context structs.
- Port existing inline heuristics into `react.rs` and `web_routes.rs` modules.
- Add tests proving no regression in existing generic resolver behavior.

## Phase 2 (parity-first frameworks)

Implement highest-value frameworks first:

1. React
2. Express
3. Django
4. FastAPI
5. Rails

For each framework:
- add detection tests
- add resolution tests
- add ambiguity/negative tests

## Phase 3 (breadth + performance)

- Add additional frameworks (Laravel, Spring, ASP.NET, Swift ecosystems).
- Cache framework detection per project/session.
- Optionally parallelize framework resolver execution when context allows.

## Testing strategy

## Unit tests

- per framework rule tests with fixture nodes
- confidence scoring tests
- ambiguity handling tests

## Integration tests

- end-to-end project fixtures (one per framework)
- assert created edges and expected traversal results (`callers`, `callees`, `impact`)

## Regression tests

- ensure baseline non-framework projects do not regress in precision/recall

## Recommended crate/API changes

- Add `frameworks` module to `codegraph-resolution` with public trait + registry types.
- Add `ResolverConfig.framework_resolvers: Option<Vec<String>>` for allowlisting.
- Add telemetry counters in `ResolutionStats`:
  - `framework_attempts`
  - `framework_resolved`
  - `framework_ambiguous`
  - per-framework resolved counts

## Migration and compatibility notes

- Keep existing generic behavior as default fallback while framework registry is introduced.
- Gate advanced framework resolvers behind config flags if needed for conservative rollout.
- Document framework support matrix in one canonical Rust doc to avoid TS/Rust drift.

## Definition of done

- Rust resolver has trait-based framework plugin architecture.
- Top five framework resolvers implemented with integration fixtures.
- MCP callers/callees/impact outcomes measurably improved on framework-heavy repos.
- Resolver metrics expose per-framework performance and ambiguity rates.

