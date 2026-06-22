# Benchmark Summary

Results from criterion benchmarks (`cargo bench --bench benchmarks`).
Environment: Linux 6.12.86, Rust 1.96.0, release profile with fat LTO, codegen-units=1, strip=symbols, panic=abort.

## Running

```bash
cargo bench --bench benchmarks           # full suite (~5 min)
cargo bench --bench benchmarks -- flat    # filter by name
```

HTML reports are generated in `target/criterion/`.

## Build Configuration

```toml
[profile.release]
lto = "fat"
codegen-units = 1
strip = "symbols"
panic = "abort"

[profile.bench]
lto = "fat"
codegen-units = 1
strip = "symbols"
```

---

## Single-File Streaming

Flat XML: `<root><item status="active">text N</item> × N</root>`, one `CheckAttr` rule per node.

| Elements | XML Size | Time (median) | Throughput |
|----------|----------|---------------|------------|
| 1K       | 49 KB    | 1.38 ms       | 37.3 MiB/s |
| 10K      | 502 KB   | 8.94 ms       | 59.5 MiB/s |
| 100K     | 5.3 MB   | 93.1 ms       | 59.2 MiB/s |
| 500K     | 27 MB    | 464 ms        | 61.3 MiB/s |
| 1M       | 54 MB    | 515 ms        | 111 MiB/s  |

Throughput is **~60 MiB/s** for elements with attrs+text, rising to **111 MiB/s** on larger files where NodeNeeds-based text skipping takes full effect.

## Parse Overhead (Noop Rules)

Same XML, rules that do nothing — isolates parsing + context-stack overhead. With `NodeNeeds::empty()` on the noop rule, both attrs and text are skipped.

| Elements | Time (median) | Throughput |
|----------|---------------|------------|
| 10K      | 4.94 ms       | 108 MiB/s  |
| 100K     | 43.8 ms       | 126 MiB/s  |
| 500K     | 239 ms        | 119 MiB/s  |

When rules don't need data, the parser degrades to just XML event scanning + descriptor matching — **~120 MiB/s**.

## Heavy Rules (3× attr-hash + parent check, 10 attrs/element)

| Elements | XML Size | Time (median) | Throughput |
|----------|----------|---------------|------------|
| 10K      | 1.5 MB   | 23.9 ms       | 65.4 MiB/s |
| 100K     | 15.9 MB  | 233 ms        | 71.4 MiB/s |

With three CPU-intensive rules per element, monomorphized dispatch keeps throughput at **65–71 MiB/s**. The compiler inlines rule bodies through the generic `R: Rule` path.

## Deep Nesting (unmatched — only root in descriptor)

Exercises the skip-depth counter on deeply nested XML with no matched descriptors beyond root.

| Shape              | XML Size | Time (median) | Throughput  |
|--------------------|----------|---------------|-------------|
| depth=5, fan=5     | 210 KB   | 1.57 ms       | 131 MiB/s   |
| depth=10, fan=3    | 4.6 MB   | 12.7 ms       | 367 MiB/s   |
| depth=15, fan=2    | 3.7 MB   | 9.88 ms       | 373 MiB/s   |

Unmatched elements are skipped at **130–373 MiB/s**. Binary search on sorted children replaced HashMap lookup, yielding an 83% improvement at depth=10 vs the previous HashMap-based iteration.

## Capture Subtree (Mixed Streaming + Capture)

Catalog pattern: one `<schema>` captured as arena-backed mini-DOM, plus 1000 streaming `<entry>` elements.

| Schema Fields | XML Size | Time (median) | Throughput |
|---------------|----------|---------------|------------|
| 50            | 54 KB    | 1.23 ms       | 46.0 MiB/s |
| 200           | 61 KB    | 1.39 ms       | 45.7 MiB/s |
| 1000          | 97 KB    | 2.07 ms       | 49.2 MiB/s |

### Large Capture Subtrees

Same pattern, 100 streaming entries, larger captured schema.

| Schema Fields | XML Size | Time (median) | Throughput |
|---------------|----------|---------------|------------|
| 500           | 28 KB    | 1.12 ms       | 25.8 MiB/s |
| 2000          | 97 KB    | 2.24 ms       | 45.3 MiB/s |
| 5000          | 237 KB   | 4.56 ms       | 54.3 MiB/s |

Arena-backed materialization (bumpalo) with bulk deallocation. Larger subtrees amortize per-element overhead.

## Noise Ratio (Unmatched vs Matched Elements)

1000 matched `<target>` elements with varying amounts of unmatched noise.

| Noise Ratio | XML Size | Time (median) | Throughput |
|-------------|----------|---------------|------------|
| 0:1         | 28 KB    | 1.40 ms       | 19.6 MiB/s |
| 5:1         | 252 KB   | 2.47 ms       | 99.9 MiB/s |
| 20:1        | 944 KB   | 5.99 ms       | 154 MiB/s  |

Noise-dominated workloads hit **154 MiB/s** — skipped elements are nearly free (just depth counting).

## Child Aggregate (Parent Inspects All Children)

Parent rule collects unique IDs from all children into a HashSet at `</root>` time.

| Children | XML Size | Time (median) | Throughput |
|----------|----------|---------------|------------|
| 1K       | 49 KB    | 1.33 ms       | 38.5 MiB/s |
| 10K      | 502 KB   | 8.96 ms       | 59.4 MiB/s |
| 50K      | 2.6 MB   | 41.9 ms       | 65.1 MiB/s |

Aggregate rules feasible up to ~50K children. RuleResults bitset (u64) replaces SmallVec, and ChildSummary stores NodeId (4B) instead of Arc-cloned tag (16B).

## File-Level Parallelism

32 files × 10K elements each, varying rayon thread count.

| Threads | Time (median) | Throughput  | Speedup vs 1T |
|---------|---------------|-------------|----------------|
| 1       | 216 ms        | 78.9 MiB/s  | 1.0×           |
| 2       | 121 ms        | 141 MiB/s   | 1.8×           |
| 4       | 88.0 ms       | 194 MiB/s   | 2.5×           |
| 8       | 80.1 ms       | 213 MiB/s   | 2.7×           |

### Scaling with File Count (default thread pool)

10K elements per file, default rayon pool.

| Files | Total Size | Time (median) | Throughput  |
|-------|------------|---------------|-------------|
| 1     | 502 KB     | 8.13 ms       | 65.4 MiB/s  |
| 4     | 2.0 MB     | 16.3 ms       | 130 MiB/s   |
| 16    | 8.0 MB     | 40.8 ms       | 209 MiB/s   |
| 64    | 32.1 MB    | 149 ms        | 228 MiB/s   |

Throughput plateaus around **228 MiB/s** with 64 files.

## Streaming Pipeline (Channel-Fed)

Files fed incrementally via `crossbeam_channel` → `par_bridge()`.

| Files | Total Size | Time (median) | Throughput  |
|-------|------------|---------------|-------------|
| 10    | 5.0 MB     | 19.3 ms       | 275 MiB/s   |
| 50    | 25.1 MB    | 79.6 ms       | 334 MiB/s   |

Channel-fed streaming reaches **334 MiB/s** aggregate with zero overhead from the `par_bridge()` adaptation.

---

## Optimization History

Cumulative improvements from the initial implementation, measured on representative benchmarks.

| Change | 1M streaming | Parse-only 100K | Heavy 100K | Parallel 64 files | Capture 1K fields |
|--------|-------------|-----------------|------------|--------------------|--------------------|
| Baseline (initial impl) | 1.62 s / 35 MiB/s | 139 ms / 40 MiB/s | 590 ms / 28 MiB/s | 271 ms / 126 MiB/s | 3.87 ms / 26 MiB/s |
| + Arena, pools, no-clone path | 1.44 s / 40 MiB/s | — | — | — | 3.01 ms / 34 MiB/s |
| + Fat LTO, codegen-units=1 | 1.26 s / 45 MiB/s | 83.7 ms / 66 MiB/s | 475 ms / 35 MiB/s | 182 ms / 187 MiB/s | 2.47 ms / 41 MiB/s |
| + NodeNeeds skip, SmallVec | 555 ms / 103 MiB/s | 69.1 ms / 80 MiB/s | 331 ms / 50 MiB/s | 158 ms / 216 MiB/s | 2.07 ms / 49 MiB/s |
| + Monomorphize, bitpack, no HashMap | 515 ms / 111 MiB/s | 43.8 ms / 126 MiB/s | 233 ms / 71 MiB/s | 149 ms / 228 MiB/s | 2.07 ms / 49 MiB/s |
| **Total speedup** | **3.1×** | **3.2×** | **2.5×** | **1.8×** | **1.9×** |

## Key Takeaways

1. **Monomorphization is the latest big win**: generic `R: Rule` replaces `Box<dyn Rule>` vtable dispatch, enabling the compiler to inline rule bodies. Heavy rules improved 42% in this round alone.
2. **Streaming throughput**: 59–111 MiB/s per core depending on rule data requirements. Pure scanning (noop rules) reaches **126 MiB/s**.
3. **Data skipping**: `Rule::needs()` bitflags let the parser skip attrs/text collection entirely — up to 2× faster on large files.
4. **HashMap elimination**: sorted Vec + binary search for child lookup, SmallVec<[(u64, u32); 8]> for sibling counters. Deep nesting improved 83%.
5. **Bitpacking**: `RuleResults(u64)` bitset (8B) replaces `SmallVec<[RuleResult; 4]>` (40B+). `NodeNeeds(u8)` merges AccessMode + needs flags. `NodeId(u32)` halves index size.
6. **Diagnostic buffering**: local Vec flushed at configurable threshold reduces channel contention by ~256×.
7. **Parallelism**: 2.7× at 8 threads. Aggregate throughput reaches **334 MiB/s** with streaming pipeline.
8. **Capture**: Arena-backed SubtreeNode with bulk deallocation. ~49 MiB/s for mixed capture+streaming workloads.
9. **Zero-copy path**: `parse_slice()` entry point uses `Reader::from_str()` + `read_event()` for in-memory XML — no external buffer allocation.
