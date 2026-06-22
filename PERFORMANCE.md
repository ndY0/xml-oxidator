# Performance Optimization Roadmap

Deep analysis of the `xml-oxydizer` codebase for maximum throughput.
Findings ordered by effort tier, estimated impact noted per item.

---

## Architecture Overview

The pipeline is: **FileReader** (XML parse) -> **XmlWorker** (rule execution) -> **Collector** (result assembly) -> **Diagnostics**.

Communication uses Tokio mpsc/broadcast channels. Files are parsed asynchronously via `quick-xml`'s async reader, matched against a descriptor `Tree`, and workloads are dispatched to workers that run test/fold/assert rules.

---

## Tier 1 — Low Effort (hours, no structural changes)

### 1.1 `Node::rules()` clones every rule on every match

**File:** `rulebuilder.rs:356-358`
**Impact:** Critical — every matching XML element triggers N heap allocations (one `clone_box()` per rule).

```rust
// Current: clones all boxed trait objects
pub fn rules(&self) -> Vec<Box<dyn Rule>> {
    self.rules.clone()
}
```

Each `clone_box()` calls `Box::new(self.clone())` — a fresh heap allocation per rule per element. With 10,000 child elements and 1 rule, that is 10,000 unnecessary allocations.

**Fix:** Store rules as `Arc<[Box<dyn Rule>]>` and return a cheap `Arc::clone()`. Rules are never mutated on the `Node` side; mutation only happens inside the worker's private copy. Alternatively, since workers need mutable rules for `fold()`, clone once per workload dispatch (already done), but stop cloning inside `Node` — move the clone to the call site in `handle_path_match` where it is actually needed, and have `rules()` return `&[Box<dyn Rule>]`.

---

### 1.2 `Tree::children()` allocates a HashMap on every call

**File:** `rulebuilder.rs:299-305`
**Impact:** High — called on every XML start tag to check descriptor matches.

```rust
pub fn children<'b>(&self, node: &'b Node) -> HashMap<Path, &Node> {
    node.nodes.iter()
    .filter_map(|(path,index)| {
        self.nodes.get(*index).and_then(|node| {Some((path.clone(), node))})
    }).collect()
}
```

Every call allocates a HashMap and clones every `Path` key. The tree structure is immutable after build — this result is always the same.

**Fix:** Return `&HashMap<Path, usize>` (already stored as `Node::nodes`) and resolve indices at the call site, or cache the resolved children in the Tree during build. Simplest immediate fix:

```rust
pub fn child(&self, node: &Node, path: &Path) -> Option<&Node> {
    node.nodes.get(path).and_then(|&idx| self.nodes.get(idx))
}
```

This replaces the full HashMap construction with a single O(1) lookup, which is all the callers actually need (`match_child_swap_descriptor` and `collect_missing_path_children`).

---

### 1.3 `workload_counters.sort()` on every progress message

**File:** `collector.rs:112`
**Impact:** Medium — O(n log n) per progress message when O(1) is possible.

```rust
workload_counters.push(workload_counter);
workload_counters.sort();
```

The completion check verifies that counters `[0, 1, 2, ..., total-1]` are all present. A sorted vec works but is wasteful.

**Fix:** Replace `Vec<u8>` with a simple counter. Since workload_counters are sequential u8 values, track `received_count: u8` and a `max_counter: u8`. Increment `received_count` on each progress. Completion is `received_count == total_workload_count`. If out-of-order delivery is a concern, use a `u128` bitset (supports up to 128 workloads) or `HashSet<u8>`:

```rust
// Instead of Vec<u8> + sort:
received_count += 1;
let completed = total_workload_count
    .is_some_and(|total| received_count == total + 1);
```

---

### 1.4 Redundant `String::from_utf8_lossy` conversions in hot path

**File:** `filereader.rs:254, 266, 320, 401`
**Impact:** Medium — allocates a new String for every XML tag, even when only comparing against a known byte pattern.

```rust
let tag_path = Path(String::from_utf8_lossy(tag.name().as_ref()).into());
// ...
if reader_context.current_descriptor.path().0.as_bytes() == tag.name().as_ref()
```

Line 266 correctly compares bytes — but line 254 already converted to a String. The conversion on line 254 is only needed if the tag actually matches a descriptor path. Defer the conversion.

**Fix:** Compare using raw bytes first (already done for the direct match). For child lookups, add a `child_by_bytes(&self, node: &Node, bytes: &[u8])` method to Tree that compares against `path.0.as_bytes()` without allocating a String/Path. Only construct the `Path` after confirming a match.

---

### 1.5 `attrs` HashMap cloned multiple times per element

**File:** `filereader.rs:425-458`
**Impact:** Medium — `attrs.clone()` is called 2-3 times per matched element (FullNodeView, PartialNodeView, children sender).

**Fix:** Build one `Arc<HashMap<String, String>>` and share it across views. Or since `PartialNodeView` already clones attrs into its own HashMap, use `Arc` once and pass `Arc::clone()`.

---

### 1.6 Missing capacity hints on Vecs and HashMaps

**Files:** Throughout
**Impact:** Low-Medium — realloc/rehash overhead.

- `collector.rs:66` — `HashMap::new()` → use `HashMap::with_capacity(expected_files)`
- `filereader.rs:421` — `HashMap::new()` for attrs → `HashMap::with_capacity(tag.attributes().count())`
- `collector.rs:123-124` — `Vec::new()` for results/counters could pre-size
- `rulebuilder.rs:268-269` — `Tree::new()` with empty Vec

---

### 1.7 Increase `BufReader` capacity for XML parsing

**File:** `filereader.rs:196`
**Impact:** Low-Medium — default is 8KB, XML files may benefit from 32-64KB.

```rust
let reader = BufReader::new(src);
```

**Fix:**

```rust
let reader = BufReader::with_capacity(64 * 1024, src);
```

Reduces syscall frequency for large files. Benchmark to find optimal size.

---

### 1.8 Unnecessary `text.clone().into_inner()` in Event::Text

**File:** `filereader.rs:341`
**Impact:** Low — clones byte buffer before conversion.

```rust
String::from_utf8_lossy(&text.clone().into_inner())
```

`Event::Text` already owns the data. The `.clone()` creates a redundant copy. Use `text.into_inner()` directly (requires taking ownership of the event text, which is possible since `event` is `&mut Event`). Alternatively, use `text.as_ref()` to avoid any copy:

```rust
String::from_utf8_lossy(text.as_ref())
```

---

### 1.9 Path string formatting in `fold` operations

**Files:** `xmlworker.rs:147-149`, `collector.rs:214, 229`
**Impact:** Low-Medium — `format!("{}/{}", acc, path.0)` in an iterator fold allocates a new String per path segment.

```rust
payload.path.iter().fold(String::new(), |acc, curr| format!("{}/{}", acc, curr.0))
```

**Fix:** Use a single pre-allocated String with `push_str`:

```rust
let mut path_str = String::with_capacity(payload.path.iter().map(|p| p.0.len() + 1).sum());
for p in &payload.path {
    path_str.push('/');
    path_str.push_str(&p.0);
}
```

This path string is computed identically in multiple places. Compute it once and pass it around.

---

### 1.10 Use `std::sync::Mutex` where async lock isn't needed

**File:** `filereader.rs:21` — `ctx: Arc<Mutex<HashMap<...>>>` in XmlWorkload
**Impact:** Low-Medium — `tokio::sync::Mutex` has ~2x the overhead of `std::sync::Mutex` due to its fairness guarantees and async bookkeeping.

If the critical section is short (just inserting/looking up in a HashMap), `std::sync::Mutex` is faster. Only use `tokio::sync::Mutex` when you need to hold the lock across `.await` points.

Audit each `Mutex` usage:
- `global_context` in `filereader.rs:463` — lock is held across no `.await`, use `std::sync::Mutex`
- `FullNodeView` in `XmlWorkload::events` — lock is held across `.await` in rules, keep `tokio::sync::Mutex`

---

## Tier 2 — Medium Effort (1-3 days, localized refactors)

### 2.1 Replace `Vec<Path>` HashMap keys with path IDs

**Files:** All — `HashMap<Vec<Path>, ...>` used everywhere
**Impact:** Critical — hashing `Vec<Path>` is O(depth) and involves hashing each String. This is the most pervasive allocation pattern.

Every path lookup hashes a vector of heap-allocated Strings. Descriptor trees are fixed after build.

**Fix:** Assign a numeric `PathId` (u32/u64) to each unique path during tree construction. Use `HashMap<PathId, ...>` or `Vec` indexed by PathId. Path interning eliminates all the String hashing and Vec allocation for lookups:

```rust
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct PathId(u32);

pub struct PathInterner {
    paths: Vec<Vec<Path>>,
    index: HashMap<Vec<Path>, PathId>,
}
```

After interning, all path-keyed maps become `HashMap<PathId, ...>` — a single u32 hash per lookup.

---

### 2.2 Replace `broadcast` channel with `watch` for text content

**File:** `filereader.rs:433`, `rulebuilder.rs:602-604`
**Impact:** Medium — `broadcast::channel` maintains a buffer and supports multiple receivers with independent read cursors. For single-value text content, this is overkill.

Text content per element is typically a single string. `tokio::sync::watch` is cheaper — one slot, no buffer management, no lagged-receiver bookkeeping.

**Fix:**

```rust
let (text_sender, text_receiver) = watch::channel(String::new());
```

If multiple text events per element are expected (interleaved text nodes), accumulate into the watch value rather than streaming separate messages.

---

### 2.3 Avoid `Arc<Mutex<FullNodeView>>` where single-owner suffices

**File:** `filereader.rs:48, 451`, `xmlworker.rs:139`
**Impact:** Medium — every node view is wrapped in `Arc<Mutex<>>`. The Mutex is locked once by the worker and never contested.

The view is sent through a channel and consumed by a single worker task. There is no shared ownership at runtime.

**Fix:** Send `FullNodeView` directly through the channel (owned, not shared). The worker takes exclusive ownership. Rules receive `&mut FullNodeView` instead of locking a Mutex:

```rust
// Channel type becomes:
Sender<FullNodeView>  // instead of Sender<Arc<Mutex<FullNodeView>>>

// Rule trait becomes:
fn fold(&mut self, view: &mut FullNodeView, ctx: &HashMap<...>) -> ...
```

This eliminates Arc reference counting and Mutex lock/unlock per node per rule.

---

### 2.4 Sequential rule execution instead of `join_all`

**File:** `xmlworker.rs:140-143`
**Impact:** Medium — `join_all` on rule folds creates a `Vec<Pin<Box<dyn Future>>>`, polls each, and manages waker registration.

```rust
join_all(
    payload.rules.iter_mut()
    .map(|rule| rule.fold(view.clone(), payload.ctx.clone()))
).await;
```

If rules are lightweight (attribute checks, simple folds), the overhead of future creation and polling exceeds the actual work. Each rule also `Arc::clone`s the view and context.

**Fix:** Run rules sequentially when there are few rules (benchmark threshold, likely < 4):

```rust
for rule in &mut payload.rules {
    rule.fold(&mut view, &ctx).await;
}
```

This eliminates N-1 Arc clones of the view and context, plus the join_all orchestration overhead. Only use `join_all` if rules perform actual I/O or heavy computation.

---

### 2.5 Replace `Arc<Mutex<file_receiver>>` with work partitioning

**File:** `init.rs:144`
**Impact:** Medium — all reader tasks contend on a single Mutex to receive the next file.

```rust
let rx = Arc::new(Mutex::new(file_receiver));
```

Every reader must acquire the Mutex, receive, then release — serializing file dispatch.

**Fix:** Pre-distribute files across readers using separate channels:

```rust
let file_senders: Vec<Sender<FileInfo<S>>> = (0..reader_count)
    .map(|_| mpsc::channel(queue_size))
    .collect();
// Round-robin or load-balance file dispatch externally
```

Or use a single `mpsc::Receiver` properly — Tokio's mpsc supports multiple consumers if you clone the receiver... but it doesn't. Instead, use `async_channel::Receiver` which supports multi-consumer, or use `tokio::sync::Semaphore` based work-stealing.

---

### 2.6 `SmallVec` for short path vectors

**Files:** Throughout — `Vec<Path>` used for paths that are typically 2-4 elements deep
**Impact:** Low-Medium — avoids heap allocation for short paths.

```toml
[dependencies]
smallvec = "1"
```

```rust
type PathVec = SmallVec<[Path; 4]>;
```

Most XML descriptor paths are shallow (root -> child -> grandchild). SmallVec stores up to 4 elements inline (stack-allocated), only spilling to heap for deeper paths.

---

### 2.7 `Box<str>` or `Arc<str>` instead of `String` for `Path`

**File:** `rulebuilder.rs:672`
**Impact:** Medium — `Path(String)` is cloned extensively. Strings are never mutated after construction.

```rust
#[derive(PartialEq, Eq, Hash, Clone, Debug)]
pub struct Path(pub String);
```

**Fix:**

```rust
#[derive(PartialEq, Eq, Hash, Clone, Debug)]
pub struct Path(pub Arc<str>);
```

`Arc<str>::clone()` is a single atomic increment (no heap alloc). `String::clone()` allocates and copies. Given that `Path` is cloned in `Tree::children()`, `handle_path_match`, `collect_missing_path_children`, `FullDiagnostic::from`, etc., this compounds heavily.

---

### 2.8 Deduplicate redundant `Arc::clone` of `workload_counter_seq`

**File:** `filereader.rs:275-276`
**Impact:** Low — `workload_counter_seq` is cloned twice (once as `workload_id_seq`, once passed directly). Both are the same Arc.

```rust
Arc::clone(&workload_counter_seq),  // as workload_id_seq parameter
// ...
workload_counter_seq,  // moved as workload_counter_seq parameter
```

The function signature has both `workload_id_seq: Arc<AtomicU8>` and `workload_counter_seq: Arc<AtomicU8>` — these are the same object. Merge into one parameter.

---

## Tier 3 — High Effort (days-weeks, architectural changes)

### 3.1 Zero-copy XML parsing with byte-level path matching

**Impact:** Critical for throughput — current approach converts every tag to a String, then compares. `quick-xml` supports zero-copy parsing where events reference the input buffer.

**Current flow:**
1. `read_event_into_async(&mut read_buf)` — writes event bytes into `read_buf`
2. `String::from_utf8_lossy(tag.name().as_ref())` — copies bytes into a new String
3. `Path(string)` — wraps in Path
4. Compares against descriptor paths (which are also Strings)

**Optimized flow:**
1. Same read
2. Compare `tag.name().as_ref()` directly against `descriptor.path.as_bytes()` (already done in one place at line 266, but not consistently)
3. Only allocate Path/String when a match is confirmed and the path needs to be stored

Store descriptor paths as `Vec<u8>` (or `Box<[u8]>`) internally, expose `as_str()` for display. All matching uses byte comparison — no UTF-8 conversion in the hot path.

---

### 3.2 Arena allocation for node views

**Impact:** High — current code creates a new `FullNodeView` (with HashMap, Receiver, etc.) per matched element. These objects are short-lived — created during parsing, consumed by a worker, then dropped.

An arena allocator (e.g., `bumpalo`) can batch-allocate these objects from a single contiguous region, eliminating per-object malloc/free overhead and improving cache locality.

```rust
use bumpalo::Bump;

let arena = Bump::new();
// Allocate views from the arena
let view = arena.alloc(FullNodeView::new(...));
// Reset after each file
arena.reset();
```

Caveat: requires careful lifetime management to avoid sending arena-allocated objects across tasks. Could use per-worker arenas.

---

### 3.3 Replace dynamic dispatch rules with enum dispatch

**Impact:** Medium-High — `Box<dyn Rule>` uses vtable dispatch for `fold()` and `assert()`. This prevents inlining and causes indirect branch prediction misses.

If the set of rule types is known at compile time, use an enum:

```rust
enum AnyRule {
    Concrete(ConcreteRule<...>),
    // future variants
}
```

This enables direct dispatch (match on enum variant), which the compiler can inline. The `dyn Rule` approach also forces heap allocation (`Box`), while enum dispatch can be stack-allocated.

If the rule type set is truly open, consider `enum_dispatch` crate for a compromise.

---

### 3.4 Batch workload dispatch

**Impact:** Medium — currently each matching XML element sends a separate workload through the channel. Channel send/recv has overhead (~50-100ns per operation with Tokio mpsc).

For files with thousands of matching elements (as in the test: 10,000 children), batch N elements into a single workload message:

```rust
pub struct XmlWorkloadBatch {
    pub file_id: u64,
    pub file: String,
    pub workloads: Vec<SingleWorkload>,
}
```

Reduces channel operations by a factor of N (batch size). Workers process the batch in a tight loop without channel overhead between elements.

---

### 3.5 Profile-guided optimization (PGO) and LTO

**Impact:** Medium — no code changes required, build configuration only.

```toml
[profile.release]
lto = "fat"
codegen-units = 1
panic = "abort"
```

Then use PGO:

```bash
# Build instrumented binary
RUSTFLAGS="-Cprofile-generate=/tmp/pgo-data" cargo build --release

# Run representative workload
./target/release/xml-oxydizer <representative-input>

# Merge profiles
llvm-profdata merge -o /tmp/pgo-data/merged.profdata /tmp/pgo-data

# Build optimized binary
RUSTFLAGS="-Cprofile-use=/tmp/pgo-data/merged.profdata" cargo build --release
```

Typical gains: 10-20% throughput improvement from better inlining and branch layout decisions.

---

### 3.6 Rethink the channel topology

**Current topology:**
- N readers share 1 file_receiver (Mutex-contended)
- Each reader round-robins across M worker senders
- Each worker sends to 1 collector
- 1:1 mapping of readers to collectors (but workers fan-in)

**Problems:**
- The Mutex on file_receiver serializes readers
- Round-robin doesn't account for worker load (a worker stuck on a heavy rule starves)
- Collector per reader means results for the same file can arrive at different collectors (if a file's workloads go to workers mapped to different collectors)

**Proposed topology:**
- Single dispatcher task distributes files to readers via per-reader channels (no Mutex)
- Single shared work queue (crossbeam or async-channel multi-consumer) for all workers
- Single collector (or sharded by file_id hash) to avoid cross-collector coordination
- Backpressure via bounded channels already exists — tune buffer sizes based on profiling

---

### 3.7 Consider `rayon` for CPU-bound rule execution

**Impact:** Varies — if rules are compute-heavy (regex matching, deep tree traversal), offloading to a Rayon thread pool avoids blocking Tokio's cooperative scheduler.

```rust
let result = tokio::task::spawn_blocking(move || {
    rules.par_iter_mut().for_each(|rule| {
        rule.fold_sync(&view, &ctx);
    });
}).await;
```

Only beneficial if rules are CPU-intensive. For simple attribute checks, the task-spawn overhead exceeds the gain.

---

## Tier 4 — Dependency-level Optimizations

### 4.1 Trim `tokio-util` features

**File:** `Cargo.toml:13`

```toml
tokio-util = { version = "0.7.18", features = ["full", "io", "io-util"] }
```

`"full"` enables everything including codecs, framing, compat layers. Only `CancellationToken` is used. Reduce to:

```toml
tokio-util = { version = "0.7.18", features = ["sync"] }  # only CancellationToken
```

Reduces compile time and binary size (less monomorphization).

---

### 4.2 Remove `itertools` if only used for `chunks`

**File:** `Cargo.toml:9`, `init.rs:4`

`itertools` is used for `.chunks()` in `init.rs:147`. This can be replaced with `std::slice::chunks()` after collecting to a Vec first, or with a manual chunking loop. Removes a dependency.

---

### 4.3 Replace `educe` with manual `Debug` impls

**File:** `Cargo.toml:7`

`educe` is a proc-macro crate used to skip Debug on closure fields. This can be done with manual `impl Debug`:

```rust
impl<Acc, R> fmt::Debug for ConcreteRule<Acc, R> where Acc: Debug {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConcreteRule")
            .field("name", &self.name)
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}
```

Removes proc-macro compile-time cost.

---

## Measurement Priorities

Before implementing, establish baselines:

1. **Allocation profiling** — Run with `dhat` (already in dev-deps). The `test-heap` feature is set up. Priority findings will be rules cloning (1.1) and HashMap allocations (1.2, 2.1).

2. **Flamegraph** — `cargo flamegraph` on the integration test. Will reveal whether time is spent in channel operations, Mutex contention, or rule execution.

3. **Latency distribution** — Instrument channel send/recv to identify bottleneck stages. If readers are blocked waiting for workers, increase worker count. If collectors are idle, reduce collector count.

4. **Throughput test** — Create a benchmark with a 100MB+ XML file to measure events/sec. The current test uses 10,000 elements from a mock — representative but not stress-level.

Recommended implementation order for maximum impact per effort:

1. **1.1** (stop cloning rules) + **1.2** (stop allocating children HashMap) — these dominate allocation
2. **1.3** (counter sort) + **1.9** (path string formatting) — easy quick wins
3. **2.3** (remove Arc<Mutex> from views) + **2.4** (sequential rule fold) — reduce synchronization overhead
4. **2.7** (Arc<str> for Path) + **2.1** (path interning) — reduce hashing/cloning across the board
5. **3.5** (PGO/LTO) — free performance from the compiler

---

## Deep Dive — Streaming Parent/Children View Sharing

This section dissects the channel-based mechanism that allows rules to observe a node's children (downward) or its parent's view (upward) during streaming XML parsing. This is the most architecturally complex part of the codebase, and the one where performance and correctness are most tightly coupled.

### How it works today

**Children (parent observes children):**

1. A node declares `map_children: Some(vec![vec![Path("root"), Path("child")]])` at build time — "I want to see child elements matching this path".
2. When the reader matches this node (`handle_path_match` in `filereader.rs:431-492`):
   - For each declared child path, a `broadcast::channel<PartialNodeView>` is created (line 440).
   - Receivers go into the `FullNodeView::children` HashMap, which is sent to the worker.
   - Senders go into `CurrentViewContext::children_sender`, pushed onto the `current_view` stack.
3. When a descendant element is later matched (line 472-484):
   - The code **linearly scans the entire `current_view` stack**, flat-mapping all `children_sender` entries.
   - For each sender whose declared path matches the current element's path, a new `PartialNodeView` is constructed and sent.
4. The parent's rule (running in a worker) calls `view.children().get_mut(&path)` to get the broadcast receiver, then loops `child_receiver.recv().await` to consume each child as it arrives.
5. The loop terminates when the broadcast sender is dropped — which happens when `CurrentViewContext` is popped from the stack at the parent's `</end>` tag.

**Parent (child observes parent):**

1. A node declares `map_view: true` at build time — "expose my view to descendants".
2. When matched (line 462-469), a `PartialNodeView` is inserted into `global_context: Arc<Mutex<HashMap<Vec<Path>, PartialNodeView>>>`.
3. All child workloads for the same file share this `Arc<Mutex<...>>`. The child rule locks it and looks up the parent.

### Problem S.1 — `broadcast` is the wrong channel for 1:1 children streaming

**Files:** `filereader.rs:440`, `rulebuilder.rs:603`
**Impact:** High (performance) + Medium (correctness risk)

Each parent→child relationship creates a `broadcast::channel`. Broadcast channels are designed for 1-to-many fan-out: they maintain a ring buffer, track per-subscriber read cursors, and handle `Lagged` errors when slow consumers fall behind.

In this codebase, there is exactly **one producer** (the reader, via `CurrentViewContext::children_sender`) and exactly **one consumer** (the parent rule, via `FullNodeView::children`). This is a textbook 1:1 pattern where `mpsc` (or even `oneshot` for single-child) is optimal.

The broadcast overhead vs mpsc:
- Broadcast maintains a shared ring buffer behind an internal `Mutex`. Every `send()` acquires this lock.
- Broadcast `recv()` must check for `Lagged` on every call, doing a cursor comparison against the ring's tail.
- Memory: broadcast pre-allocates a ring of `highwatermark` slots. With `highwatermark = 10` and 1,000 parent nodes, that's 10,000 pre-allocated `PartialNodeView` slots sitting empty.

**Correctness risk:** `broadcast::channel` is a **ring buffer that overwrites old entries**. If a parent has more children than `highwatermark`, the channel wraps and early children are lost. The receiver gets `RecvError::Lagged(n)`, meaning `n` messages were silently dropped. In the test (`integration_test.rs:26-37`), the rule does `child_receiver.recv().await` in a loop — a `Lagged` error would break the loop via the `Err(_err) => false` arm, silently reporting the check as failed rather than flagging a data integrity violation.

With 10,000 child elements in the test and `highwatermark = 10`, this **will** overflow if the rule doesn't consume children fast enough. The current test likely passes because the rule is trivial (attribute check) and keeps up, but under load or with heavier rules, children are silently dropped.

**Fix:** Replace with `mpsc::channel` (bounded or unbounded depending on backpressure strategy):

```rust
// Instead of broadcast::channel
let (tx, rx) = mpsc::channel::<PartialNodeView>(highwatermark);
```

`mpsc` provides true backpressure: when the buffer is full, `send().await` suspends the sender (the XML reader) until the receiver consumes. No data loss. Lower per-message overhead. No ring buffer bookkeeping.

If backpressure against the reader is undesirable (it would stall parsing), use an unbounded channel and rely on memory as the natural limit — still cheaper than broadcast.

---

### Problem S.2 — `PartialNodeView` constructed redundantly per child match

**File:** `filereader.rs:456-460, 475-479`
**Impact:** Medium — allocates N copies where 1 would suffice.

When a child element is matched, the code constructs a `PartialNodeView` in multiple places:

1. **Line 456-460:** Built for `map_view` insertion into `global_context` — clones `attrs`, creates `Arc::new(text_sender.subscribe())`.
2. **Line 475-479:** Built for each ancestor whose `children_sender` matches — clones `attrs` again, calls `text_sender.subscribe()` again (creating another broadcast receiver), wraps in another `Arc`.

Each construction does:
- `attrs.clone()` — HashMap allocation + clone of every key-value String pair
- `text_sender.subscribe()` — creates a new broadcast receiver (internal Mutex lock, cursor setup)
- `Arc::new(...)` — heap allocation for the Arc control block

For a file with 10,000 matched children, this is at least 20,000 HashMap clones and 20,000 broadcast subscriptions.

**Fix:** Build the `PartialNodeView` once and use `Arc<PartialNodeView>` for sharing:

```rust
let partial_view = Arc::new(PartialNodeView::new(
    attrs,  // move, not clone
    inner_view_index,
    text_receiver,  // single receiver, shared via Arc
));
// global_context and children_sender both get Arc::clone(&partial_view)
```

This requires changing `PartialNodeView` text from a per-instance receiver to a shared mechanism (see S.4).

---

### Problem S.3 — Linear scan of ancestor stack per child match

**File:** `filereader.rs:472-484`
**Impact:** Medium — O(stack_depth × children_per_ancestor) per matched element.

```rust
let children_sender_results = reader_context.current_view.iter()
    .flat_map(|view_context| view_context.children_sender.iter())
    .filter(|(desired_path, _)| current_path.eq(*desired_path))
    .map(|(_, children_sender)| children_sender.send(...));
```

For every child match, this scans the entire `current_view` stack (depth of open ancestors) and all their declared children paths. In a deeply nested XML with many `map_children` declarations, this is wasteful.

**Fix:** Maintain a `HashMap<Vec<Path>, Vec<&BroadcastSender<PartialNodeView>>>` as an index. When a `CurrentViewContext` is pushed, register its children paths in the index. When popped, remove them. Lookup becomes O(1):

```rust
if let Some(senders) = reader_context.children_path_index.get(&current_path) {
    for sender in senders {
        sender.send(partial_view.clone());
    }
}
```

---

### Problem S.4 — `Arc<Receiver<String>>` on `PartialNodeView` is unusable

**File:** `rulebuilder.rs:642-644, 658-660`
**Impact:** Correctness/design issue, with performance side-effects from the workaround.

```rust
pub struct PartialNodeView {
    text: Arc<Receiver<String>>,  // broadcast::Receiver
    // ...
}

impl CommonNodeView for PartialNodeView {
    fn text(&self) -> &Receiver<String> {
        &self.text  // returns &Receiver, but recv() needs &mut self
    }
}
```

`broadcast::Receiver::recv()` requires `&mut self`. Behind `Arc`, there is no way to get `&mut` without interior mutability (`Mutex` or `RefCell`). The `CommonNodeView::text()` trait returns `&Receiver<String>` — an immutable reference. This means the text of a `PartialNodeView` **cannot be received**. It is either unused in practice or requires `unsafe`/casting workarounds.

Each `PartialNodeView` creates a new `text_sender.subscribe()` (line 459, 478) — a broadcast receiver that can never be consumed. This is pure overhead: a broadcast subscription allocates an internal state tracker and registers with the sender's subscriber list.

**Fix (short-term):** If parent rules don't need to stream children's text, remove the text field from `PartialNodeView` entirely. If they do, use `Arc<Mutex<Receiver<String>>>` or change the design:

**Fix (proper):** Replace the broadcast text mechanism with `tokio::sync::watch` for text content. Text is typically a single value (the element's text content, fully known at `</end>` time). A `watch` channel stores one value, allows multiple readers via `watch::Receiver::borrow()` (no `&mut self` needed), and costs a fraction of broadcast:

```rust
pub struct PartialNodeView {
    text: watch::Receiver<Option<String>>,
    // ...
}
// text() returns a Ref guard, no mut needed
```

---

### Problem S.5 — `global_context` Mutex is a serialization bottleneck for parent access

**File:** `filereader.rs:67, 462-469`, `rulebuilder.rs:571`
**Impact:** High under concurrent workloads.

```rust
pub global_context: Arc<Mutex<HashMap<Vec<Path>, PartialNodeView>>>
```

Every child workload receives `Arc::clone` of this same Mutex. Every rule that accesses parent context does:

```rust
// From the test (integration_test.rs:47)
let ctx = ctx.lock().await;
let is_parent_ok = match ctx.get(&vec![Path("child".into())]) { ... };
```

If 10,000 child elements are processed by N worker tasks concurrently, all N tasks serialize on this single `tokio::Mutex`. Under `join_all` in `xmlworker.rs:141-143`, multiple rules within the same workload also serialize on it.

The data being read (parent views) is **write-once, read-many**: a parent's `PartialNodeView` is inserted once (at match time) and then only read by descendants. This is the textbook case for `RwLock`, not `Mutex`:

```rust
pub global_context: Arc<RwLock<HashMap<Vec<Path>, PartialNodeView>>>
```

But even `RwLock` has overhead from the reader count tracking. Since the map is append-only during parsing, the truly optimal approach is:

```rust
// Insert with Mutex/write-lock (rare, one per matched parent)
// Read with a lock-free snapshot (frequent, every child rule)
pub global_context: Arc<DashMap<Vec<Path>, Arc<PartialNodeView>>>
```

Or simpler: since the parent is always matched before its children in document order, insert the `PartialNodeView` into the context **before** dispatching any child workloads. The children can then receive a pre-built `Arc<PartialNodeView>` directly in their `XmlWorkload`, eliminating the Mutex entirely for reads:

```rust
pub struct XmlWorkload {
    // ...
    pub parent_view: Option<Arc<PartialNodeView>>,  // injected at dispatch
}
```

---

### Problem S.6 — Rules hold `tokio::Mutex` across long `.await` chains

**File:** `integration_test.rs:20-41`
**Impact:** High — blocks all other rules on the same view for the duration of child streaming.

```rust
let test_root = |view: Arc<Mutex<FullNodeView>>, _ctx| async move {
    let mut view = view.lock().await;        // <-- lock acquired
    let is_attr_ok = view.attr("test")...;
    let is_child_attr_ok = match view.children().get_mut(&...) {
        Some(child_receiver) => {
            loop {
                match child_receiver.recv().await {  // <-- awaiting with lock held
                    // ...
                }
            }
        }
    };
};
```

The Mutex guard on `FullNodeView` is held for the **entire duration of children streaming**. Since children arrive as the XML reader processes subsequent elements, the lock is held across potentially thousands of `.await` points — one per child element.

`join_all` in `xmlworker.rs:141-143` runs all rules on the same path concurrently. If two rules both try to `view.lock().await`, they serialize completely. The second rule cannot even read an attribute until the first rule has finished streaming all children.

This is the canonical "hold Mutex across .await" anti-pattern in async Rust. `tokio::sync::Mutex` technically supports it (unlike `std::sync::Mutex`), but it destroys concurrency between rules on the same path.

**Fix:** Separate attribute access from children streaming. Read attributes eagerly (short critical section), then stream children without the lock:

```rust
let test_root = |view: Arc<Mutex<FullNodeView>>, _ctx| async move {
    // Short lock: read attrs + extract children receiver
    let (is_attr_ok, mut child_rx) = {
        let mut view = view.lock().await;
        let attr_ok = view.attr("test").is_some_and(|v| v == "value");
        let rx = view.children_mut().remove(&path);  // take ownership
        (attr_ok, rx)
    };
    // Lock released — stream children without holding it
    let is_child_attr_ok = match child_rx { ... };
};
```

Better yet, redesign `FullNodeView` to not need a Mutex at all (see section 2.3 — send it as an owned value, not `Arc<Mutex<>>`). If the view is owned by the workload task, there is no contention.

---

### Problem S.7 — Channel infrastructure created for every matched element, even without children/parent needs

**File:** `filereader.rs:433, 440`
**Impact:** Low-Medium — wasted allocation for leaf nodes.

Every matched element gets:
- A `broadcast::channel` for text (`text_sender`, `text_receiver`)
- A `broadcast::channel` per declared `map_children` entry
- A `PartialNodeView` if `map_view` is true
- A `CurrentViewContext` pushed to the stack

For a leaf `<child>` element with no `map_children` and `map_view: false`, the text broadcast channel and empty `children_sender` HashMap are still created. With 10,000 leaf children, that is 10,000 unnecessary broadcast channels.

**Fix:** Only create text/children channels when the descriptor actually uses them:

```rust
let text_channel = if reader_context.current_descriptor.needs_text() {
    let (tx, rx) = broadcast::channel(highwatermark);
    Some((tx, rx))
} else {
    None
};
```

This requires making `FullNodeView::text` optional. Rules that don't inspect text never pay for the channel.

---

### Alternative architecture: Event-sourced views with deferred materialization

The current design eagerly creates channels and views for every matched element, then streams children through broadcast channels. An alternative inverts this:

1. **The reader emits raw events** into a single channel per path: `(ElementId, EventKind, data)`.
2. **Workers materialize views on demand**: when a rule needs children, it subscribes to the event stream and filters for relevant child events. No pre-built channels per child path.
3. **Parent context is a shared append-only log**, not a mutable HashMap. Children index into it by parent ID.

This collapses the N broadcast channels (one per parent-child relationship) into a single event stream, and defers view construction to the point of consumption. Elements without rules incur zero overhead beyond the raw event.

This is a significant rewrite, but it eliminates the fundamental tension between the streaming parser (which processes elements sequentially) and the channel-per-relationship model (which pre-allocates communication infrastructure for relationships that may never be queried).

---

## Expected Throughput Impact by Tier

These are estimates based on the allocation and synchronization patterns identified above. Actual gains depend on workload characteristics (file size, tree depth, rule complexity, worker count). The estimates assume the integration test profile: 10,000 child elements, simple attribute-check rules, 1 file.

Gains within a tier compound multiplicatively (e.g., two 10% improvements yield ~19%, not 20%). Gains across tiers stack — Tier 2 improvements compound on top of a Tier 1-optimized baseline.

### Tier 1 — Low Effort

| Item | Estimated Gain | Dominant Mechanism |
|------|----------------|-------------------|
| 1.1 Rules cloning | 15-25% | Eliminates 10k+ heap allocs (clone_box) per file |
| 1.2 children() HashMap | 10-20% | Eliminates HashMap alloc + Path clones per tag event |
| 1.3 Sort elimination | 1-3% | Removes O(n log n) per progress message |
| 1.4 UTF-8 conversion | 5-10% | Avoids String alloc on every non-matching tag |
| 1.5 attrs clone | 3-5% | Removes 2-3 HashMap clones per matched element |
| 1.6 Capacity hints | 2-5% | Fewer realloc/rehash cycles |
| 1.7 BufReader size | 2-5% | Fewer syscalls for large files |
| 1.8 text clone | 1-2% | Avoids one byte-buffer copy per text event |
| 1.9 Path formatting | 2-5% | Avoids N String allocs per path construction |
| 1.10 std::sync::Mutex | 3-5% | Cheaper lock for short critical sections |

**Combined Tier 1 estimate: ~30-50% throughput improvement.**

Dominated by allocation reduction (1.1, 1.2). Most items are mechanical fixes under 10 lines each. A profiling session with `dhat` will confirm which items dominate.

### Tier 2 — Medium Effort

| Item | Estimated Gain | Dominant Mechanism |
|------|----------------|-------------------|
| 2.1 Path interning | 15-25% | Eliminates Vec<Path> hashing everywhere |
| 2.2 watch vs broadcast | 5-10% | Lighter channel for single-value text |
| 2.3 Remove Arc\<Mutex\> views | 10-15% | Eliminates Mutex lock/unlock + Arc refcount per rule fold |
| 2.4 Sequential rules | 5-10% | Removes join_all overhead + N-1 Arc clones |
| 2.5 Work partitioning | 5-10% | Removes Mutex contention on file dispatch |
| 2.6 SmallVec | 3-5% | Stack-allocates short path vectors |
| 2.7 Arc\<str\> for Path | 5-10% | Atomic increment replaces memcpy on every Path clone |

**Combined Tier 2 estimate: ~30-50% additional improvement** (on top of Tier 1 baseline).

The biggest win is path interning (2.1) — it touches every HashMap lookup in the system. Combined with Arc<str> (2.7), it transforms the most common allocation pattern.

### Tier 3 — High Effort

| Item | Estimated Gain | Dominant Mechanism |
|------|----------------|-------------------|
| 3.1 Zero-copy parsing | 10-20% | Eliminates all String allocs in the parse loop |
| 3.2 Arena allocation | 10-15% | Batch alloc/dealloc for views |
| 3.3 Enum dispatch | 5-10% | Enables inlining of rule fold/assert |
| 3.4 Batch dispatch | 10-15% | Reduces channel operations by batch factor |
| 3.5 PGO + LTO | 10-20% | Compiler-level optimization, no code changes |
| 3.6 Channel topology | 5-15% | Better load distribution, less contention |
| 3.7 Rayon offload | 0-20% | Only if rules are CPU-heavy |

**Combined Tier 3 estimate: ~40-60% additional improvement** (on top of Tiers 1+2 baseline).

PGO/LTO (3.5) is notable: it gives 10-20% for zero code changes — just build configuration. Should be done regardless of other work.

### Streaming view fixes (S.1-S.7)

| Item | Estimated Gain | Dominant Mechanism |
|------|----------------|-------------------|
| S.1 mpsc instead of broadcast | 5-10% + correctness fix | Eliminates ring buffer overhead, prevents data loss |
| S.2 Deduplicate PartialNodeView | 5-10% | Eliminates 2-3x redundant HashMap/receiver allocs per child |
| S.3 Children path index | 2-5% | O(1) lookup replaces O(depth × children) scan |
| S.4 Fix unusable text receiver | 1-3% + correctness fix | Removes dead broadcast subscriptions |
| S.5 RwLock / inject parent | 5-15% | Eliminates serialization on parent context reads |
| S.6 Don't hold Mutex across await | 5-15% | Enables actual rule concurrency within a path |
| S.7 Lazy channel creation | 3-5% | Skip channel alloc for elements that don't use them |

**Combined streaming fixes estimate: ~20-40% additional improvement**, with **S.1 also fixing a latent correctness bug** (silent child data loss under load).

### Total theoretical throughput envelope

Applying all tiers multiplicatively: `1.4 × 1.4 × 1.5 × 1.3 ≈ 3.8x` — roughly **3-4x** throughput improvement over the current implementation. The first 2x comes almost entirely from Tiers 1+2 (allocation and synchronization reduction). The remainder requires architectural changes (Tier 3 + streaming fixes).

These are upper-bound estimates. Real gains are always lower due to Amdahl's law — some time is irreducible (actual XML parsing in `quick-xml`, kernel I/O, etc.). Profiling after each tier determines whether diminishing returns have set in.
