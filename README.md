# xml-oxydizer

A Rust library for **streaming XML validation** with synchronous rule execution and file-level parallelism.

xml-oxydizer parses XML in a single forward pass using [`quick-xml`](https://crates.io/crates/quick-xml), evaluates user-defined rules at element close time, and parallelizes across files via [rayon](https://crates.io/crates/rayon). It supports two access modes per element — **streaming** (O(depth) memory) and **subtree capture** (mini-DOM materialization) — letting you choose the right memory/power tradeoff at each point in your XML schema.

## Features

- **Single-pass streaming parser** — O(depth) memory for streaming nodes; no full DOM required.
- **Selective subtree capture** — opt into full DOM access per-node when rules need deep queries (find, find_all, descendants).
- **File-level parallelism** — rayon thread pool processes files concurrently; diagnostics stream via crossbeam channel.
- **Lazy file loading** — stream factories defer I/O until the worker thread is ready.
- **Parent/children/sibling access** — rules can read ancestor attributes, text, and completed children summaries from the context stack.
- **Memory-safe capture limits** — configurable byte cap per captured subtree to prevent OOM.
- **Declarative schema builder** — fluent API (and optional `build_tree!` macro) to declare expected XML structure, access modes, and rules.
- **Zero-copy slice parsing** — `parse_slice` avoids buffered I/O for in-memory XML data.

## Architecture

```
FileInfo (lazy stream factory)
  → rayon thread pool (one file per task)
    → parse_file(): context-stack streaming + selective subtree capture
      → Rules evaluate synchronously at </end> time
        → Diagnostics streamed via crossbeam channel
```

### Access Modes

Each descriptor node declares an access mode via `NodeNeeds`:

| Mode | Memory | Capability |
|---|---|---|
| **Streaming** | O(depth) | Attrs, text, children summaries, ancestor access |
| **CaptureSubtree** | O(subtree) | Full mini-DOM with `find`, `find_all`, `descendants` |

### Module Overview

| Module | Purpose |
|---|---|
| `tree::builder` | Fluent `TreeBuilder` API for constructing descriptor trees |
| `tree::descriptor` | `DescriptorNode`, `DescriptorTree`, `NodeNeeds` bitflags |
| `tree::path` | `PathSegment` (Arc\<str\>-backed), `NodeId`, path formatting |
| `rule` | `NodeAccess` trait (unified view interface), `Rule` trait |
| `view` | `StreamingView`, `SubtreeView`, `SubtreeNode`, `ChildSummary` |
| `reader::parser` | Core `parse_file` / `parse_slice` event loops |
| `reader::capture` | Memory-limited `CaptureBuilder` and arena materialization |
| `reader::context` | Per-element `NodeContext` stack frames and object pools |
| `pipeline` | `FileInfo`, `PipelineConfig`, `run_pipeline` / `run_pipeline_streaming` |
| `diagnostic` | `Diagnostic` and `Severity` types |
| `error` | `BuilderError`, `ReaderError`, `PipelineError` |

## Usage

Add to your `Cargo.toml`:

```toml
[dependencies]
xml-oxydizer = "0.2"
```

### Defining Rules

Implement the `Rule` trait for each validation check:

```rust
use xml_oxydizer::diagnostic::{Diagnostic, Severity};
use xml_oxydizer::rule::{NodeAccess, Rule};
use xml_oxydizer::tree::descriptor::NodeNeeds;

struct CheckVersion {
    expected: &'static str,
}

impl Rule for CheckVersion {
    fn name(&self) -> &str {
        "check_version"
    }

    fn evaluate(&self, node: &dyn NodeAccess) -> Vec<Diagnostic> {
        match node.attr("version") {
            Some(v) if v == self.expected => vec![],
            other => vec![Diagnostic {
                rule_name: self.name().to_owned(),
                severity: Severity::Error,
                message: format!("expected version=\"{}\", got {:?}", self.expected, other),
                element_path: node.path().to_vec(),
                element_index: node.element_index() as u32,
            }],
        }
    }

    fn needs(&self) -> NodeNeeds {
        NodeNeeds::ATTRS
    }
}
```

For rules that need full subtree access, set the `CAPTURE` flag:

```rust
struct ValidateSchema;

impl Rule for ValidateSchema {
    fn name(&self) -> &str { "validate_schema" }

    fn needs(&self) -> NodeNeeds {
        NodeNeeds::all() | NodeNeeds::CAPTURE
    }

    fn evaluate(&self, node: &dyn NodeAccess) -> Vec<Diagnostic> {
        match node.subtree() {
            Some(subtree) => {
                if subtree.find("field").is_none() {
                    vec![Diagnostic {
                        rule_name: self.name().to_owned(),
                        severity: Severity::Error,
                        message: "schema must contain at least one <field>".to_owned(),
                        element_path: node.path().to_vec(),
                        element_index: node.element_index() as u32,
                    }]
                } else {
                    vec![]
                }
            }
            None => vec![],
        }
    }
}
```

### Building the Descriptor Tree

Use the fluent `TreeBuilder` API to declare your expected XML structure:

```rust
use std::sync::Arc;
use xml_oxydizer::tree::builder::TreeBuilder;
use xml_oxydizer::rule::Rule;

let tree = TreeBuilder::new("catalog")
    .streaming()
    .rule(Box::new(CheckVersion { expected: "3" }) as Box<dyn Rule>)
    .node("schema")
        .capture_subtree()
        .rule(Box::new(ValidateSchema) as Box<dyn Rule>)
        .done()
    .node("entry")
        .streaming()
        .node("detail")
            .streaming()
            .done()
        .done()
    .capture_limit(8 * 1024 * 1024)  // 8 MB per captured subtree
    .build()
    .expect("invalid descriptor tree");

let tree = Arc::new(tree);
```

### Running the Pipeline

```rust
use std::io::Cursor;
use crossbeam_channel::bounded;
use xml_oxydizer::pipeline::{FileInfo, PipelineConfig, run_pipeline};
use xml_oxydizer::rule::Rule;

let xml_data = br#"<catalog version="3">
    <schema><field name="sku" type="string"/></schema>
    <entry><detail>Widget A</detail></entry>
</catalog>"#;

let (diag_tx, diag_rx) = bounded(1024);

let files = vec![FileInfo {
    filename: "catalog.xml".to_owned(),
    descriptors: Arc::clone(&tree),
    stream_factory: Box::new(move || {
        Box::new(Cursor::new(xml_data.to_vec())) as Box<dyn std::io::Read + Send>
    }),
}];

let errors = run_pipeline(files, diag_tx, &PipelineConfig::default());
assert!(errors.is_empty());

let diagnostics: Vec<_> = diag_rx.try_iter().collect();
for diag in &diagnostics {
    eprintln!("[{}] {}: {}", diag.severity, diag.rule_name, diag.message);
}
```

### Streaming Pipeline (Channel-Fed)

For scenarios where file discovery and validation should overlap:

```rust
use crossbeam_channel::bounded;
use xml_oxydizer::pipeline::{FileInfo, PipelineConfig, run_pipeline_streaming};

let (file_tx, file_rx) = bounded(16);
let (diag_tx, diag_rx) = bounded(4096);

// Producer thread sends files as they are discovered
let producer = std::thread::spawn(move || {
    for path in discover_xml_files() {
        file_tx.send(make_file_info(path)).unwrap();
    }
});

// Pipeline processes files as they arrive
let errors = run_pipeline_streaming(file_rx, diag_tx, &PipelineConfig::default());
producer.join().unwrap();
```

### `build_tree!` Macro (Optional)

Enable the `macros` feature for a declarative syntax:

```toml
[dependencies]
xml-oxydizer = { version = "0.2", features = ["macros"] }
```

```rust
use xml_oxydizer::build_tree;

let tree = build_tree!(
    "catalog" streaming {
        CheckVersion { expected: "3" },
        "schema" capture {
            ValidateSchema {}
        },
        "entry" streaming {
            "detail" {}
        }
    }
)?;
```

## Build & Test

```bash
# Build
cargo build

# Run all tests
cargo test

# Run integration tests only
cargo test --test integration_test

# Run macro tests (requires macros feature)
cargo test --test macro_test --features macros

# Heap profiling test (with dhat)
cargo test --test integration_test --features test-heap

# Lint
cargo clippy

# Format
cargo fmt

# Generate documentation
cargo doc --open
```

## Benchmarks

Run the full benchmark suite with [Criterion](https://crates.io/crates/criterion):

```bash
cargo bench
```

Results are written to `target/criterion/` with HTML reports.

### Performance Preview

Measured on a single core (streaming mode, element-level rules):

| Scenario | Elements | Time | Throughput |
|---|---|---|---|
| Flat streaming (attr check) | 10,000 | ~8.5 ms | ~66 MB/s |
| Flat streaming (attr check) | 100,000 | ~86 ms | ~65 MB/s |
| Flat streaming (attr check) | 500,000 | ~471 ms | ~60 MB/s |
| Parse only (noop rule) | 100,000 | ~43 ms | ~130 MB/s |
| Parse only (noop rule) | 500,000 | ~213 ms | ~133 MB/s |
| Heavy rules (3x hash, 10 attrs) | 10,000 | ~23 ms | ~140 MB/s |
| Heavy rules (3x hash, 10 attrs) | 100,000 | ~232 ms | ~140 MB/s |
| Deep nesting (5 levels, 5 children) | 3,905 | ~1.1 ms | ~186 MB/s |
| Deep nesting (10 levels, 3 children) | 88,573 | ~12.7 ms | ~366 MB/s |

Key performance characteristics:

- **O(depth) memory** for streaming nodes — constant regardless of file size.
- **Near-linear scaling** with element count.
- **Parallel speedup** across files via rayon (near-linear with core count).
- **Undeclared elements are skipped** with minimal overhead (1 depth counter per nested level).
- **Object pooling** for `NodeContext` and attribute vectors minimizes allocations.
- **Bump allocation** for captured subtrees — single deallocation at subtree close.

## Minimum Supported Rust Version

Rust **edition 2024** (requires rustc 1.85+).

## License

See [Cargo.toml](Cargo.toml) for package metadata.
