# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

`xml-oxydizer` — a Rust library for streaming XML validation with synchronous rule execution. Implements the "Hybrid Streaming Context Stack with Selective Subtree Capture" architecture (IMPROVEMENT.md §5.4). Parses XML via `quick-xml` (sync), evaluates rules synchronously per file, and parallelizes across files via rayon.

## Build & Test

```bash
cargo build
cargo test                                        # all tests
cargo test --test integration_test                 # integration test only
cargo test --test integration_test --features test-heap  # with dhat heap profiling
cargo clippy                                      # lint
```

Rust edition 2024. No CI config — use standard `cargo fmt` and `cargo clippy`.

## Architecture

Hybrid streaming/capture model with file-level parallelism:

```
FileInfo (lazy stream factory)
  → rayon thread pool (one file per task)
    → parse_file(): context-stack streaming + selective subtree capture
      → Rules evaluate synchronously at </end> time
        → Diagnostics streamed via crossbeam channel
```

### Access Modes

Each descriptor node declares an **AccessMode**:
- **Streaming** — context-stack model: attrs + text + children summaries + parent reference. O(depth) memory.
- **CaptureSubtree** — buffers XML events, materializes a mini-DOM at `</end>`. O(subtree_size) memory.

### Modules (under `src/`)

- **tree/path.rs** — `PathSegment` (Arc<str>-backed), `NodeId`, path formatting.
- **tree/descriptor.rs** — `AccessMode`, `DescriptorNode`, `DescriptorTree` with O(1) child lookup via `child_tag_index`.
- **tree/builder.rs** — `TreeBuilder` with consuming-self fluent API. Build-time validation: access mode compatibility, nested capture detection, duplicate paths.
- **rule.rs** — `NodeAccess` trait (unified view interface), `Rule` trait (sync, stateless).
- **view.rs** — `StreamingView`, `SubtreeView`, `SubtreeNode` (mini-DOM with find/descendants), `ChildSummary`.
- **reader/context.rs** — `NodeContext` (per-element stack frame).
- **reader/capture.rs** — `CaptureBuffer` with memory limit, `materialize()` builds `SubtreeNode` tree.
- **reader/parser.rs** — `parse_file()` core event loop with modal streaming/capture switching.
- **diagnostic.rs** — `Diagnostic`, `Severity`.
- **pipeline.rs** — `FileInfo` (lazy loading), `PipelineConfig`, `run_pipeline`/`run_pipeline_streaming`.
- **error.rs** — `BuilderError`, `ReaderError`, `PipelineError`.

### Key Types

- `DescriptorTree` / `DescriptorNode` — declarative schema of XML structure with per-node access modes and rules
- `Rule` trait — `evaluate(&self, node: &dyn NodeAccess) -> Vec<Diagnostic>`, synchronous
- `NodeAccess` trait — unified interface for both `StreamingView` and `SubtreeView`; index-based ancestor access
- `StreamingView` — references context stack slice, O(1) parent access by index
- `SubtreeView` — wraps `SubtreeNode` mini-DOM with DOM query methods (find, find_all, descendants)
- `ChildSummary` — tag, attrs, text, index, rule_results; pushed to parent context at `</end>`
- `FileInfo` — filename + `Arc<DescriptorTree>` + lazy `Box<dyn FnOnce() -> Box<dyn Read + Send> + Send>`

### Parent/Children Access

Single unified mechanism via context stack:
- **Parent access**: `ancestor_attrs(level)` indexes into the context stack (0 = parent, 1 = grandparent)
- **Children access**: `children_summaries()` returns completed child summaries in document order
- **Sibling access**: parent's `children_summaries` accumulates as children process (previous siblings visible)

## Key Documentation Files

- `IMPROVEMENT.md` — architectural analysis; §5.4 describes the implemented hybrid model
- `PERFORMANCE.md` — performance optimization roadmap
- `SUGGESTIONS.md` — code style improvements
- `TODO.md` — feature roadmap
