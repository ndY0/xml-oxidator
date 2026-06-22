# Benchmark Summary

Results from criterion benchmarks (`cargo bench --bench benchmarks`).
Environment: Linux 6.12.86, Rust 1.96.0, release profile with fat LTO + codegen-units=1.

## Running

```bash
cargo bench --bench benchmarks           # full suite (~5 min)
cargo bench --bench benchmarks -- flat    # filter by name
```

HTML reports are generated in `target/criterion/`.

## Build Configuration

```toml
[profile.bench]
lto = "fat"
codegen-units = 1
```

---

## Single-File Streaming

Flat XML: `<root><item status="active">text N</item> × N</root>`, one `CheckAttr` rule per node.

| Elements | XML Size | Time (median) | Throughput |
|----------|----------|---------------|------------|
| 100K     | 5.3 MB   | 116 ms        | 47.4 MiB/s |
| 500K     | 27 MB    | 649 ms        | 43.8 MiB/s |
| 1M       | 54 MB    | 555 ms        | 103 MiB/s  |

The 1M-element result benefits most from the needs-based data skipping: the `CheckAttr` rule only declares `ATTRS`, so text collection is skipped entirely for leaf elements.

## Parse Overhead (Noop Rules)

Same XML, rules that do nothing — isolates parsing + context-stack overhead. With `NodeNeeds::empty()` on the noop rule, both attrs and text are skipped.

| Elements | Time (median) | Throughput |
|----------|---------------|------------|
| 10K      | 7.62 ms       | 69.8 MiB/s |
| 100K     | 69.1 ms       | 79.7 MiB/s |
| 500K     | 291 ms        | 97.5 MiB/s |

When rules don't need data, the parser degrades to just XML event scanning + descriptor matching — **~80-100 MiB/s**.

## Heavy Rules (3× attr-hash + parent check, 10 attrs/element)

| Elements | XML Size | Time (median) | Throughput |
|----------|----------|---------------|------------|
| 10K      | 1.5 MB   | 34.9 ms       | 44.8 MiB/s |
| 100K     | 15.9 MB  | 331 ms        | 50.3 MiB/s |

30% faster than previous due to SmallVec for rule results and sibling counter pooling. Rule evaluation is still the bottleneck but with less allocation overhead around it.

## Deep Nesting (unmatched — only root in descriptor)

Exercises the skip-depth counter on deeply nested XML with no matched descriptors.

| Shape              | XML Size | Time (median) | Throughput  |
|--------------------|----------|---------------|-------------|
| depth=5, fan=5     | 210 KB   | 1.98 ms       | 104 MiB/s   |
| depth=10, fan=3    | 4.6 MB   | 23.1 ms       | 201 MiB/s   |
| depth=15, fan=2    | 3.7 MB   | 12.8 ms       | 287 MiB/s   |

Unmatched elements are skipped at **100–287 MiB/s**. These elements only increment/decrement `skip_depth`.

## Capture Subtree (Mixed Streaming + Capture)

Catalog pattern: one `<schema>` captured as arena-backed mini-DOM, plus 1000 streaming `<entry>` elements.

| Schema Fields | XML Size | Time (median) | Throughput |
|---------------|----------|---------------|------------|
| 50            | 54 KB    | 1.59 ms       | 35.6 MiB/s |
| 200           | 61 KB    | 1.34 ms       | 47.6 MiB/s |
| 1000          | 97 KB    | 2.07 ms       | 49.2 MiB/s |

### Large Capture Subtrees

Same pattern, 100 streaming entries, larger captured schema.

| Schema Fields | XML Size | Time (median) | Throughput |
|---------------|----------|---------------|------------|
| 500           | 28 KB    | 1.34 ms       | 21.7 MiB/s |
| 2000          | 97 KB    | 2.69 ms       | 37.8 MiB/s |
| 5000          | 237 KB   | 5.42 ms       | 45.6 MiB/s |

Arena-backed materialization with bulk deallocation. Larger subtrees amortize overhead.

## Noise Ratio (Unmatched vs Matched Elements)

1000 matched `<target>` elements with varying amounts of unmatched noise.

| Noise Ratio | XML Size | Time (median) | Throughput |
|-------------|----------|---------------|------------|
| 0:1         | 28 KB    | 1.32 ms       | 20.9 MiB/s |
| 5:1         | 252 KB   | 2.78 ms       | 88.5 MiB/s |
| 20:1        | 944 KB   | 6.69 ms       | 138 MiB/s  |

Noise-dominated workloads hit **138 MiB/s** — skipped elements are nearly free.

## Child Aggregate (Parent Inspects All Children)

Parent rule collects unique IDs from all children into a HashSet at `</root>` time.

| Children | XML Size | Time (median) | Throughput |
|----------|----------|---------------|------------|
| 1K       | 49 KB    | 1.25 ms       | 41.2 MiB/s |
| 10K      | 502 KB   | 8.49 ms       | 62.7 MiB/s |
| 50K      | 2.6 MB   | 42.1 ms       | 65.1 MiB/s |

20% faster than previous due to SmallVec rule_results and optimized ChildSummary.

## File-Level Parallelism

32 files × 10K elements each, varying rayon thread count.

| Threads | Time (median) | Throughput  | Speedup vs 1T |
|---------|---------------|-------------|----------------|
| 1       | 236 ms        | 72.0 MiB/s  | 1.0×           |
| 2       | 132 ms        | 129 MiB/s   | 1.8×           |
| 4       | 92.5 ms       | 184 MiB/s   | 2.6×           |
| 8       | 85.5 ms       | 199 MiB/s   | 2.8×           |

### Scaling with File Count (default thread pool)

10K elements per file, default rayon pool.

| Files | Total Size | Time (median) | Throughput  |
|-------|------------|---------------|-------------|
| 1     | 502 KB     | 8.01 ms       | 66.4 MiB/s  |
| 4     | 2.0 MB     | 18.3 ms       | 116 MiB/s   |
| 16    | 8.0 MB     | 42.6 ms       | 200 MiB/s   |
| 64    | 32.1 MB    | 158 ms        | 216 MiB/s   |

Throughput plateaus around **216 MiB/s** with 64 files.

## Streaming Pipeline (Channel-Fed)

Files fed incrementally via `crossbeam_channel` → `par_bridge()`.

| Files | Total Size | Time (median) | Throughput  |
|-------|------------|---------------|-------------|
| 10    | 5.0 MB     | 21.6 ms       | 246 MiB/s   |
| 50    | 25.1 MB    | 86.7 ms       | 307 MiB/s   |

Channel-fed streaming now reaches **307 MiB/s** — a major improvement from the needs-based skipping across many files.

---

## Optimization History

Cumulative improvements from the initial implementation, measured on representative benchmarks.

| Change | 1M streaming | Parse-only 500K | Parallel 64 files | Capture 1K fields |
|--------|-------------|-----------------|--------------------|--------------------|
| Baseline (initial impl) | 1.62 s / 35.2 MiB/s | 811 ms / 35.0 MiB/s | 271 ms / 126 MiB/s | 3.87 ms / 26.3 MiB/s |
| + Arena, pools, no-clone path | 1.44 s / 39.5 MiB/s | — | — | 3.01 ms / 33.8 MiB/s |
| + Fat LTO, codegen-units=1 | 1.26 s / 45.2 MiB/s | 589 ms / 48.2 MiB/s | 182 ms / 187 MiB/s | 2.47 ms / 41.2 MiB/s |
| + NodeNeeds skip, SmallVec, sibling pool | 555 ms / 103 MiB/s | 291 ms / 97.5 MiB/s | 158 ms / 216 MiB/s | 2.07 ms / 49.2 MiB/s |
| **Total improvement** | **2.9× faster** | **2.8× faster** | **1.7× faster** | **1.9× faster** |

## Key Takeaways

1. **Data skipping is the biggest win**: when rules don't need text or attrs, skipping collection saves 50%+ on large files. The `Rule::needs()` method enables this at build time.
2. **Streaming throughput**: 47–103 MiB/s per core depending on rule data requirements. Pure scanning (noop) reaches ~100 MiB/s.
3. **Memory**: O(depth) for streaming, O(subtree_size) for captures. Attrs/text skipped when unreferenced.
4. **Parallelism**: 2.8× at 8 threads. Aggregate throughput reaches **307 MiB/s** with streaming pipeline.
5. **Capture**: Arena-backed SubtreeNode with bulk deallocation. ~49 MiB/s for mixed capture+streaming workloads.
6. **SmallVec + pooling**: Eliminates per-element HashMap and Vec allocations for sibling counters and rule results.
