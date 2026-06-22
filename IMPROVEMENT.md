# Architectural Critique & Improvement Proposals

A design-level review of `xml-oxydizer`. This document examines the fundamental choices — not "how to make it faster" (see `PERFORMANCE.md`), but "is this the right shape."

---

## 1. The Core Tension: Streaming Parse vs. Relational Rules

The system parses XML as a stream (event by event, bounded memory) but evaluates rules that express *relationships* across the document tree (parent attributes, children aggregation). These goals are in conflict, and the current architecture resolves the tension with channels — broadcast channels for children, a shared Mutex HashMap for parent context.

This works, but it forces every component to pay for the bridging infrastructure whether it needs it or not, and introduces a class of problems (backpressure, data loss, Mutex contention, deadlock risk) that don't exist in simpler models.

The question to ask is: **what does streaming actually buy here?**

For a rule on `<root>` that needs to see all `<child>` elements, the rule cannot assert until every child has been parsed. The rule task sits parked on `child_receiver.recv().await`, waking once per child, folding, then parking again. It completes only when the reader reaches `</root>` and drops the sender. The rule's lifetime is identical to a non-streaming model that simply collects children during parsing and evaluates the rule at `</root>`. The streaming model gives incremental child delivery, but the fold/assert pattern consumes ALL children before producing a result — so the incremental delivery has no observable consumer-side benefit.

Streaming does buy one thing: **bounded memory per element**. A parent rule doesn't need to hold all 10,000 children in memory simultaneously — it sees them one at a time through the fold. But this advantage is undermined by the channel buffers (`highwatermark` pre-allocated slots), the `PartialNodeView` copies (each carrying a cloned `HashMap<String, String>` of attributes), and the `Arc<Mutex<FullNodeView>>` wrapping. The memory footprint of the channel infrastructure may exceed what a simple `Vec<ChildSummary>` would cost.

**Verdict:** The streaming-with-channels design is architecturally expensive for the guarantees it provides. The bounded memory property is real but narrow, and could be achieved more cheaply. The sections below propose alternatives.

---

## 2. Critique of Parent/Children Matching

### 2.1 Two mechanisms for one relationship

Parent-to-child and child-to-parent use completely different systems:

| Direction | Mechanism | Declared via | Resolved at |
|-----------|-----------|-------------|-------------|
| Parent → Children | `broadcast::channel<PartialNodeView>` | `map_children` on parent | Runtime (stack scan) |
| Child → Parent | `Arc<Mutex<HashMap>>` global context | `map_view` on parent | Runtime (Mutex lock + HashMap lookup) |

These are the same structural relationship — one edge in the XML tree — expressed through two unrelated subsystems. This means:

- Two sets of bugs to maintain (channel lifecycle vs. Mutex contention)
- Two different failure modes (broadcast `Lagged` vs. Mutex poisoning)
- The user must configure both sides independently (`map_children` on the parent, `map_view` on the parent for children to access it)
- No enforcement that the two sides are consistent

A single mechanism should handle both directions. The tree structure is known at build time — the builder has all the information needed to resolve parent/child relationships statically.

### 2.2 Absolute paths are fragile and redundant

`map_children` takes `Vec<Vec<Path>>` — full absolute paths from root:

```rust
Root::new("root", true, Some(vec![vec![Path("root".into()), Path("child".into())]]))
```

But the tree descriptor already encodes that `child` is a child of `root`. The `map_children` declaration restates what the builder structure already expresses. This creates a consistency risk: if the tree is restructured (insert a new level between root and child), the `map_children` paths become stale but compile successfully.

**The builder knows the structure. The runtime should not need to re-derive it.**

### 2.3 Runtime path matching via stack scan

When a child element is matched, the reader scans the entire `current_view` stack to find ancestors that declared interest in this child path (`filereader.rs:472-484`):

```rust
reader_context.current_view.iter()
    .flat_map(|view_context| view_context.children_sender.iter())
    .filter(|(desired_path, _)| current_path.eq(*desired_path))
```

This is O(depth × declared_children) per matched element. But the match is deterministic — the descriptor tree knows exactly which ancestor-descendant pairs exist. This should be resolved to a direct lookup at build time, not a linear scan at parse time.

### 2.4 No structural relationship beyond direct parent/child

The current model supports:
- Parent seeing its direct children (via `map_children`)
- Children seeing their parent (via `map_view` + global context)

It does **not** support:
- **Siblings** — "validate that every `<item>` is unique compared to previous `<item>` elements"
- **Ancestors beyond parent** — "check that a deeply nested element's root has a specific attribute"
- **Arbitrary descendants** — "find all `<error>` elements anywhere in the subtree"
- **Cross-file references** — "validate that an ID referenced in file A exists in file B"

Each of these would require yet another mechanism bolted onto the existing channel infrastructure. The architecture doesn't generalize.

### 2.5 Eager channel creation for relationships that may not be queried

Every matched element with `map_children` creates broadcast channels for all declared child paths, even if the rule never calls `view.children()`. Every element creates a text broadcast channel even if no rule reads text. The cost is paid upfront, not on demand.

---

## 3. Critique of the Worker/Rule Execution Model

### 3.1 One workload per descriptor path, not per element

The reader dispatches one `XmlWorkload` per unique descriptor path. All elements matching that path stream through a single `events: Receiver<Arc<Mutex<FullNodeView>>>` to the worker. The worker folds over all elements sequentially:

```rust
while let Some(view) = payload.events.recv().await {
    join_all(rules.iter_mut().map(|rule| rule.fold(view.clone(), ...))).await;
}
```

This means **parallelism is across paths, not across elements**. A file with 3 descriptor paths (root, child, grandchild) uses at most 3 workers, regardless of how many worker tasks are available. The 10,000 child elements are processed sequentially by one worker.

For CPU-bound rules, this is a bottleneck. For I/O-bound rules (the common case with channel-based view access), it means a single task alternates between waiting for views and evaluating rules — no pipeline parallelism within a path.

### 3.2 Rules are forced async even when synchronous

The `Rule::fold` signature returns `Pin<Box<dyn Future>>`:

```rust
fn fold(&mut self, view: Arc<Mutex<FullNodeView>>, ctx: ...) -> Pin<Box<dyn Future<Output = ()> + Send + Sync + '_>>;
```

Every rule, no matter how simple, must:
1. Allocate a `Box` for the future
2. Be polled by the executor
3. `await` the Mutex lock on the view
4. `await` the Mutex lock on the context (if accessing parent)

A rule that checks `view.attr("x") == "value"` — a 10-nanosecond operation — pays ~200ns of async overhead (Box alloc + poll + Mutex). Over 10,000 elements, that's 2ms of pure overhead for a rule that should take 0.1ms total.

The async requirement exists because *some* rules need to await children via broadcast channels. But most rules don't — they just inspect attributes. The architecture taxes all rules for a capability only some use.

### 3.3 `fold` / `assert` separation loses context

The rule system splits evaluation into two phases:
- `fold(&mut self, view)` — called per element, accumulates state
- `assert(&self, path) -> Diagnostic` — called once after all elements, produces result

The `fold` reduces the entire stream to a single accumulator value. The `assert` decides pass/fail based on that value. This is clean for aggregate rules ("at least one child has attribute X"), but loses per-element context:

- Which specific element failed? The fold discards element identity.
- What was the element's position in the file? Not tracked.
- What were its attributes? Already folded away.

The diagnostic can only say "the rule on path /root/child passed/failed" — not "element #47 at line 1203 has an invalid attribute." For a validation library, per-element diagnostics are often the primary output.

### 3.4 The deadlock detection is coarse

`xmlworker.rs:100-107` detects deadlocks by checking if the task channel is full and all tasks are in-flight:

```rust
if task_sender.capacity() == 0
    && task_sender.max_capacity() as u64 <= payloads_count.load(...)
{
    fatal_error_handle.trigger_fatal(FatalError::new("deadlock appeared..."));
}
```

This fires when the task queue is saturated — but that can also happen under legitimate load (burst of workloads arriving faster than rules complete). The detection conflates backpressure with deadlock. A true deadlock requires a cycle in the wait graph (task A holds resource X, waiting for resource Y held by task B, which waits for X). The current check doesn't verify any cycle — it just sees "queue full" and assumes deadlock.

---

## 4. Critique of the Builder Pattern

### 4.1 The typestate recursion is deep and hard to extend

The builder uses typestate encoding with recursive generic parents:

```
Root<InitState>
  → Child<InitState, Root<NodeAdderState>>
    → Child<InitState, Child<NodeAdderState, Root<NodeAdderState>>>
      → ...
```

Each nesting level adds a layer of generics. For a 5-level XML tree, the type is 5 generics deep. This impacts:
- **Compile time** — deep generic instantiation is quadratic in rustc
- **Error messages** — a type error on a deeply nested builder produces multi-line type signatures
- **IDE support** — autocompletion struggles with deeply recursive associated types

### 4.2 `Rc<RefCell<Tree>>` during construction is a runtime check for a static guarantee

The builder shares a mutable `Tree` via `Rc<RefCell<Tree>>`. This is a runtime borrow check (panics on double-borrow) used during a build-time-only operation where ownership is actually well-defined. The `Rc` exists so that both `Root` and `Child` can add nodes to the same tree, but the builder always transitions linearly (you can't use a builder after calling `.path()` or `.build()`). The consuming-self pattern already enforces this at compile time — the `Rc<RefCell>` adds runtime overhead for a guarantee the type system already provides.

### 4.3 Node relationships are index-based and post-hoc

Nodes are stored in a `Vec<Node>` and reference each other by index. Parent-child links are set after construction via `set_child_parent(child_index, parent_index)`. This means:
- Removing or reordering nodes invalidates all indices
- The tree cannot be validated until fully built (dangling indices)
- Node lookup is O(1) (good), but the indirection is conceptually unnecessary — an arena or petgraph-style graph would be more natural

---

## 5. Proposed Alternatives

### 5.1 Context-stack model (recommended for most cases)

Replace channels with a synchronous context stack managed by the reader. No cross-task communication for parent/children access.

**How it works:**

```
XML Reader parses event by event.
Maintains a stack of `NodeContext` — one per open matched element.

On <start>:
  Push a new NodeContext { attrs, text: String::new(), children_results: Vec }
  Parent context is stack[stack.len() - 2] — direct reference, no Mutex.

On text:
  Append to top-of-stack.text

On </end>:
  Pop the NodeContext.
  Materialize a FullView from it (attrs + text + accumulated children).
  Run all rules synchronously against the FullView.
  Produce per-element RuleResults.
  Push a ChildSummary into the parent's children_results.
```

**What this eliminates:**
- All broadcast channels (text + children)
- The `Arc<Mutex<HashMap>>` global context
- The `Arc<Mutex<FullNodeView>>` wrapping
- The async requirement on rules
- The `CurrentViewContext` stack + children_sender machinery
- The stack-scanning path match at lines 472-484

**What it preserves:**
- Streaming parse (event by event)
- Bounded memory (each element is materialized and consumed at `</end>`, not held)
- Parent access (just look up the stack)
- Children access (accumulated in parent's context during child processing)

**Trade-off:** Rules execute in the reader task, not in a worker pool. Parallelism shifts from "across paths within a file" to "across files." If files are the unit of work (the common case for validation), this is the same or better parallelism — each file gets its own reader task running rules synchronously.

For CPU-heavy rules, offload to `spawn_blocking` at the per-element level.

```rust
struct NodeContext {
    attrs: HashMap<String, String>,
    text: String,
    children: Vec<ChildSummary>,
}

struct ChildSummary {
    path: Path,
    attrs: HashMap<String, String>,
    text: String,
    rule_results: Vec<RuleResult>,
}

// Rule becomes synchronous:
trait Rule {
    fn evaluate(&self, view: &NodeView) -> RuleResult;
}

struct NodeView<'a> {
    attrs: &'a HashMap<String, String>,
    text: &'a str,
    children: &'a [ChildSummary],
    parent: Option<&'a NodeContext>,  // reference, not Arc<Mutex>
}
```

**Parent access:** `stack[stack.len() - 2]` — zero-cost borrow, always available.

**Children access:** When `</child>` fires, the child's results are pushed into `stack.last().children`. When `</parent>` fires, all children are available as `&[ChildSummary]`.

**Sibling access:** The `children` vec on the parent context accumulates as children are processed. Rule on child N can see children 0..N-1 via `parent.children` (previous siblings already processed).

This model is essentially what SAX-based validators (Schematron, custom handlers) have used for decades. It's simpler, faster, and more expressive than the channel model.

### 5.2 Lightweight DOM with arena allocation (for complex rules)

If rules need random access to arbitrary parts of the tree (ancestors, descendants, siblings, cross-references), the context-stack model becomes awkward. In that case, build a lightweight in-memory tree:

```rust
use bumpalo::Bump;

struct ArenaTree<'a> {
    arena: &'a Bump,
    nodes: Vec<ArenaNode<'a>>,
}

struct ArenaNode<'a> {
    tag: &'a str,
    attrs: &'a [(& 'a str, &'a str)],
    text: &'a str,
    parent: Option<usize>,
    children: &'a [usize],
}
```

**Pass 1:** Parse XML with `quick-xml`, build `ArenaTree`. All strings are arena-allocated (zero-copy from the parse buffer if using `quick-xml`'s borrowing reader). Memory usage: ~60-80 bytes per element + string data. A 100MB XML file with 1M elements uses ~100-160MB.

**Pass 2:** Run rules against the tree. Rules receive `&ArenaNode` with full tree access. No channels, no async, no Mutex.

**Trade-off:** Memory proportional to file size. Unsuitable for multi-GB XML files. But most validation scenarios target files that fit in memory (config files, data feeds, API responses, test fixtures). For gigabyte-scale, use the context-stack model (5.1) or a hybrid.

### 5.3 Hybrid: streaming parse with deferred rule evaluation

Keep the streaming architecture for parsing but defer rule evaluation to element completion:

```
Reader parses events, maintains a context stack.
When </end> is reached for a matched element:
  1. Materialize the element's view (attrs + text + children summaries)
  2. Send the materialized view to a worker via mpsc channel
  3. Worker runs rules synchronously against the complete view
```

**What's different from current design:**
- Views are **complete** when sent — attrs, text, and children summaries are already resolved. No broadcast channels needed.
- Rules are **synchronous** — no `.await`, no `Pin<Box<dyn Future>>`, no Mutex.
- Parent context is **injected** into the view at send time (snapshot from the stack), not looked up via global Mutex.
- Parallelism is **per element**, not per path — each completed element can be sent to any available worker.

```rust
// Materialized at </end>, sent to worker
struct CompletedElement {
    path: Vec<Path>,
    attrs: HashMap<String, String>,
    text: String,
    children: Vec<ChildSummary>,
    parent_snapshot: Option<ParentSnapshot>,  // copied from stack
}

// Rule is synchronous
trait Rule: Send {
    fn evaluate(&self, element: &CompletedElement) -> Diagnostic;
}
```

This preserves cross-element parallelism (workers process completed elements concurrently) while eliminating all channel complexity for parent/children access.

**Trade-off vs. 5.1:** Adds channel overhead for element dispatch, but gains parallelism across elements (not just across files). Best when rule evaluation is heavy relative to parsing.

### 5.4 The Hybrid: Streaming Context Stack with Selective Subtree Capture

The context-stack model (5.1) handles the common case well — attribute checks, text validation, parent/child relationships. The lightweight DOM (5.2) handles the complex case — cross-references, deep descendant queries, sibling ordering constraints. But the context stack can't look deep into a subtree's structure, and the DOM can't handle multi-GB files.

The hybrid gives each node in the descriptor tree a declared **access mode**, and the reader adapts its behavior per subtree accordingly.

#### The core idea

Most nodes in a large XML file have simple rules (check an attribute, verify text content). A few nodes have complex rules that need to see their full subtree structure — nested children, descendant counts, cross-references between sub-elements. The hybrid spends O(depth) memory on the simple parts and only materializes the complex parts.

```rust
enum AccessMode {
    Streaming,        // context-stack: attrs + text + children summaries + parent
    CaptureSubtree,   // buffer events, parse to mini-DOM at </end>
}
```

The builder declares this per node:

```rust
let tree = TreeBuilder::new()
    .node("catalog")
        .streaming()
        .rule(check_catalog_attr)
    .node("catalog/header")
        .capture_subtree()                    // complex cross-ref rules need full subtree
        .rule(validate_header_consistency)
    .node("catalog/entry")
        .streaming()                          // millions of these — keep it light
        .rule(check_entry_attr)
    .node("catalog/entry/detail")
        .streaming()
        .rule(check_detail_text)
    .build()?;
```

In this example, `catalog`, `entry`, and `detail` are streamed (bounded memory). `header` is captured — its entire subtree is buffered and parsed into a mini-DOM before rules evaluate.

#### Reader loop: modal parsing

The reader maintains a mode stack that switches between streaming and capturing:

```rust
enum ReaderMode {
    Streaming,
    Capturing {
        buffer: Vec<OwnedEvent>,   // buffered XML events
        depth: usize,              // nesting depth within captured subtree
        descriptor: NodeId,        // which descriptor node triggered capture
    },
}
```

The parse loop:

```
On <start tag>:
    if mode == Streaming:
        if descriptor.access_mode == CaptureSubtree:
            Switch to Capturing { buffer: [], depth: 1, descriptor }
            Push a placeholder NodeContext onto the context stack
            Buffer the <start> event
        else:
            Normal context-stack push (same as 5.1)
    else if mode == Capturing:
        Buffer the <start> event
        depth += 1

On </end tag>:
    if mode == Capturing:
        Buffer the </end> event
        depth -= 1
        if depth == 0:
            // Subtree complete — materialize
            let subtree = parse_buffer_to_dom(&buffer);
            let parent = context_stack.get(stack.len() - 2);
            let view = SubtreeView { subtree, parent };
            evaluate_rules(descriptor, &view);
            // Push a ChildSummary to the parent's context (for streaming parent)
            let summary = subtree.to_summary();
            context_stack.last_mut().children.push(summary);
            // Switch back
            mode = Streaming
            Pop the placeholder context
    else if mode == Streaming:
        Normal context-stack pop + evaluate rules (same as 5.1)

On text:
    if mode == Capturing:
        Buffer the text event
    else:
        Append to top-of-stack.text
```

The key property: **capture mode is transparent to the streaming parent.** When `<header>` is captured, the parent `<catalog>` still sees it as a `ChildSummary` pushed at `</header>` time. The capture is a local concern — the rest of the pipeline doesn't know or care that it happened.

#### What each mode gives rules

**Streaming mode — `StreamingView`:**

```rust
struct StreamingView<'a> {
    path: &'a [Path],
    attrs: &'a HashMap<String, String>,
    text: &'a str,
    index: usize,                             // nth occurrence of this element
    children: &'a [ChildSummary],             // completed children, in document order
    parent: Option<&'a StreamingView<'a>>,    // zero-cost stack reference
    previous_siblings: &'a [ChildSummary],    // earlier siblings from parent's children vec
}
```

Parent access: direct reference, no lock, no copy.
Children access: summaries only — path, attrs, text, rule results. No subtree structure.
Sibling access: previous siblings visible via parent's accumulated children.
Cost: O(1) per element, O(depth) total.

**Capture mode — `SubtreeView`:**

```rust
struct SubtreeView<'a> {
    root: &'a SubtreeNode<'a>,
    arena: &'a Bump,
    parent: Option<&'a StreamingView<'a>>,    // streaming parent, from context stack
}

struct SubtreeNode<'a> {
    tag: &'a str,
    attrs: &'a [(&'a str, &'a str)],
    text: &'a str,
    children: &'a [SubtreeNode<'a>],          // full child nodes, not summaries
    parent: Option<&'a SubtreeNode<'a>>,
}

impl<'a> SubtreeView<'a> {
    // Random access within the captured subtree
    fn descendants(&self) -> impl Iterator<Item = &SubtreeNode<'a>>;
    fn find(&self, path: &str) -> Option<&SubtreeNode<'a>>;
    fn find_all(&self, tag: &str) -> Vec<&SubtreeNode<'a>>;
}
```

Full DOM access within the captured subtree. XPath-like queries, descendant iteration, sibling traversal — all available. The parent is still a streaming stack reference (the capture boundary doesn't break the upward chain).

Cost: O(subtree_size) memory, allocated from an arena that is dropped after rule evaluation.

#### Unifying the rule trait across modes

Rules need to work in both modes, or be restricted to one. The trait splits access into levels:

```rust
trait NodeAccess {
    fn attrs(&self) -> &HashMap<String, String>;
    fn text(&self) -> &str;
    fn index(&self) -> usize;
    fn parent(&self) -> Option<&dyn NodeAccess>;
    fn children_summaries(&self) -> &[ChildSummary];   // always available
    fn subtree(&self) -> Option<&SubtreeView>;          // only in CaptureSubtree mode
}

trait Rule: Send + Sync {
    fn access_requirement(&self) -> AccessMode;        // Streaming or CaptureSubtree
    fn evaluate(&self, view: &dyn NodeAccess) -> Vec<Diagnostic>;
}
```

The builder validates at build time:

```rust
// In TreeBuilder::build():
for node in &declarations {
    for rule in &node.rules {
        if rule.access_requirement() == AccessMode::CaptureSubtree
            && node.access_mode == AccessMode::Streaming
        {
            return Err(BuilderError::IncompatibleAccess {
                node: node.path.clone(),
                rule: rule.name(),
                required: AccessMode::CaptureSubtree,
                provided: AccessMode::Streaming,
            });
        }
    }
}
```

Rules that only call `attrs()`, `text()`, `parent()`, or `children_summaries()` work in either mode — they don't need to declare `CaptureSubtree`. Rules that call `subtree()` must declare `CaptureSubtree`, and the builder ensures their node is in capture mode. This is a static guarantee — no runtime `Option::unwrap()` on `subtree()`.

#### Memory budget and overflow strategies

The captured subtree buffer is bounded. What happens when a subtree exceeds the budget?

**Option A — Degrade to streaming (resilient):**

```rust
if buffer.len() > capture_memory_limit {
    // Flush buffer: retrospectively process buffered events through context stack
    // Switch to streaming mode for the rest of this subtree
    // Rules that required CaptureSubtree get a degraded StreamingView + a warning diagnostic
    mode = Streaming;
    replay_buffer_as_streaming_events(&buffer, &mut context_stack);
}
```

Rules still run, but with less context. The diagnostic output includes a warning: "subtree exceeded capture limit, evaluated with partial context."

**Option B — Spill to memory-mapped file (transparent):**

```rust
if buffer.memory_usage() > in_memory_limit {
    // Spill buffer to a temp file, switch to mmap-backed buffer
    buffer.spill_to_disk()?;
    // Parsing continues transparently — the mini-DOM is built from mmap'd data
}
```

Rules see the same `SubtreeView` regardless. The DOM is backed by mmap'd memory — the OS pages in/out as needed. Peak RSS stays bounded, throughput drops slightly from I/O.

**Option C — Fail fast (strict):**

```rust
if buffer.len() > capture_memory_limit {
    return Err(ValidationError::SubtreeTooLarge {
        path: current_path.clone(),
        limit: capture_memory_limit,
        actual: buffer.len(),
    });
}
```

The user gets a clear error and must either increase the limit or restructure their descriptor to use streaming for that node. This is the simplest implementation and the most predictable.

Recommendation: start with **Option C** (fail fast). Add **Option A** later as a resilience feature. **Option B** is complex and only needed for truly adversarial inputs.

#### Nested captures

If a node at depth 2 is captured, and a node at depth 4 (within that subtree) also declares `CaptureSubtree`, the inner capture is redundant — the depth-2 buffer already includes all events for depth 4. The builder should detect and resolve this:

```rust
// In TreeBuilder::build():
for node in &declarations {
    if node.access_mode == AccessMode::CaptureSubtree {
        for ancestor in node.ancestors() {
            if ancestor.access_mode == AccessMode::CaptureSubtree {
                // Inner capture is redundant — the ancestor already captures this subtree
                // Option 1: warn and merge (node's rules run against ancestor's DOM)
                // Option 2: error (force explicit decision)
                warn!("{} is within captured subtree {} — capture is redundant", node.path, ancestor.path);
            }
        }
    }
}
```

At runtime, when the reader is already in `Capturing` mode, a nested `CaptureSubtree` descriptor is ignored — events continue buffering into the outer capture. When the outer subtree is materialized into a DOM, the inner node's rules can navigate to their portion of the DOM and evaluate.

#### Interaction with parallelism

In the hybrid model, parallelism is structured in layers:

1. **File-level parallelism** (same as 5.1): Each file gets its own reader task. N files → N tasks. This is the primary axis.

2. **Subtree-level parallelism** (new): Captured subtrees can optionally be sent to a worker pool for rule evaluation. The reader captures the buffer, sends it, and continues streaming:

```
On </end> of captured subtree:
    let buffer = take(&mut capture_buffer);
    worker_sender.send(CaptureWorkload { buffer, parent_snapshot, rules })?;
    // Reader continues streaming — doesn't wait for rule results
```

The worker parses the buffer into a DOM and evaluates rules independently. This pipelines: the reader parses the next section while the worker evaluates the previous subtree. Results are collected asynchronously.

3. **Element-level parallelism** (from 5.3): For streaming nodes with heavy rules, completed elements can also be sent to workers. But this is rarely needed — streaming rules are typically cheap.

The default recommendation: **file-level parallelism only**, with synchronous rule evaluation. Add subtree-level parallelism if profiling shows rule evaluation as the bottleneck on captured subtrees.

#### Memory characteristics by file profile

| File Profile | Streaming Memory | Capture Memory | Total Peak |
|---|---|---|---|
| 10GB, all simple rules | O(depth) ≈ 200B | 0 | ~200B |
| 10GB, one 10MB complex section | O(depth) ≈ 200B | O(10MB) | ~10MB |
| 100MB, all complex rules | O(depth) ≈ 200B | O(100MB) | ~100MB (same as full DOM) |
| 100KB config file | O(depth) ≈ 200B | O(100KB) | ~100KB |

The hybrid's memory is **O(depth + max_captured_subtree)**, which degenerates to O(depth) when no subtrees are captured (pure streaming) and to O(file_size) when the root is captured (equivalent to full DOM). The user controls the trade-off per node.

#### When to use which model

| Scenario | Recommended Model |
|---|---|
| Attribute checks, text validation, simple parent/child | **Streaming** (5.1) |
| Cross-references within a bounded section | **Hybrid** — capture that section |
| Complex rules across the entire document | **Full DOM** (5.2) if it fits in memory |
| Multi-GB files with mixed simple/complex | **Hybrid** — stream bulk, capture hotspots |
| Small config files, test fixtures | **Full DOM** (5.2) — simplest, fast enough |

#### Concrete example: validating a large data feed

A 2GB product catalog with millions of `<product>` entries and one `<schema>` header:

```xml
<catalog version="3">
    <schema>
        <field name="sku" type="string" required="true"/>
        <field name="price" type="decimal" required="true"/>
        <!-- 50 more field definitions -->
    </schema>
    <product sku="ABC123" price="29.99">
        <description>Widget</description>
    </product>
    <!-- 2 million more products -->
</catalog>
```

Rules:
1. Every `<product>` must have all required fields defined in `<schema>` (cross-reference)
2. `price` must be a valid decimal (simple attribute check)
3. `sku` must be unique across all products (aggregate/sibling check)

Descriptor:

```rust
TreeBuilder::new()
    .node("catalog")
        .streaming()
    .node("catalog/schema")
        .capture_subtree()                  // need full field list for cross-ref
        .rule(validate_schema_fields)
    .node("catalog/product")
        .streaming()
        .rule(check_required_fields)        // uses parent.captured_sibling("schema")
        .rule(check_price_format)           // simple attr check
        .rule(check_sku_uniqueness)         // aggregate: fold with HashSet<String>
    .build()?
```

Memory at any point: context stack (3 levels ≈ 300 bytes) + captured `<schema>` section (≈ 5KB). The 2 million products are streamed — each processed and discarded at `</product>`. The schema capture stays on the stack as a `ChildSummary` with the full DOM attached, accessible to product rules via `parent.children`.

Total peak memory: ~6KB for a 2GB file. Rules have full schema structure for cross-referencing, full product attributes for validation, and a fold accumulator (HashSet) for uniqueness. No channels, no Mutex, no broadcast.

---

## 6. Proposed Builder Redesign

The typestate builder is clever but the recursive generics and `Rc<RefCell>` add friction. A simpler approach:

### 6.1 Flat declarative builder

```rust
let tree = TreeBuilder::new()
    .node("root")
        .map_children(&["child"])
        .rule(my_root_rule)
    .node("root/child")
        .see_parent()
        .rule(my_child_rule)
    .build()?;
```

Relationships are declared via string paths (validated at build time). No recursive generics, no `Rc<RefCell>`. The builder collects declarations, then resolves the tree structure in `.build()`:

```rust
struct TreeBuilder {
    declarations: Vec<NodeDeclaration>,
}

struct NodeDeclaration {
    path: String,            // "root/child/grandchild"
    rules: Vec<Box<dyn Rule>>,
    see_parent: bool,
    map_children: Vec<String>,  // relative child names
}
```

`.build()` validates:
- Every path has a valid parent (no orphans)
- `map_children` targets exist as declared nodes
- `see_parent` nodes have a parent that exposes a view

Errors are returned as `Result`, not panics.

### 6.2 Relationship resolution at build time

The builder should resolve parent/children wiring statically:

```rust
struct ResolvedTree {
    nodes: Vec<ResolvedNode>,
}

struct ResolvedNode {
    path: Path,
    rules: Vec<Box<dyn Rule>>,
    parent_index: Option<usize>,
    child_indices: Vec<usize>,
    // Pre-computed: which children does this node need to observe?
    observed_children: Vec<usize>,
    // Pre-computed: does the parent observe me?
    parent_observes_me: bool,
}
```

At parse time, the reader doesn't scan stacks or match paths — it looks up `node.observed_children` and `node.parent_observes_me` directly. O(1) per element, resolved once.

---

## 7. Critique of the Collector

### 7.1 Completion detection is fragile

The collector tracks completion via sorted `Vec<u8>` workload counters (`collector.rs:253-279`). It checks that counters form a contiguous sequence `[0, 1, 2, ..., total-1]`. This assumes:

- Workload counters are assigned sequentially (true, via `AtomicU8::fetch_add`)
- Every workload reports exactly once (assumed, but a panic/abort in a worker would violate this)
- `total_workload_count` arrives (via `FileResult::Terminated`) before or after all `Progress` messages

If a worker panics after `fetch_add` but before sending `FileResult::Progress`, the counter has a gap. The collector waits forever — it never sees counter N, so the contiguous check never passes. No timeout, no fallback.

### 7.2 No file-level timeout

If a file's processing stalls (worker deadlock, channel backpressure, rule hangs on `.await`), the collector blocks indefinitely on `collector_receiver.recv()`. The `ShutdownHandle` only fires on explicit `trigger_fatal` — it doesn't detect hangs.

A per-file timeout (or per-workload timeout) would catch stalls and produce a diagnostic instead of hanging. Even a coarse "file not completed after 60 seconds" would be a safety net.

### 7.3 Results accumulate in memory until all workloads complete

`collector.rs:66` holds all intermediate results in a HashMap:

```rust
let mut results: HashMap<u64, (String, Option<u8>, Vec<u8>, Vec<RuleResult>)> = HashMap::new();
```

For a file with many paths and many elements per path, the `Vec<RuleResult>` grows unboundedly. Results can't be emitted until the file is "complete" (all workload counters present). For large files, this accumulation can be significant.

The context-stack model (section 5.1) avoids this by producing diagnostics at `</end>` time — results are emitted immediately, no accumulation.

---

## 8. Summary: What I Would Change

Ordered by architectural impact, not effort:

1. **Replace channel-based view sharing with a context stack** (section 5.1). This is the single highest-impact change. It eliminates the broadcast channels, the global Mutex, the `Arc<Mutex<FullNodeView>>` wrapping, the `CurrentViewContext` stack scanning, and the async requirement on rules. It makes parent and children access symmetric (both are stack lookups) and opens the door to sibling access for free.

2. **Make rules synchronous by default** (section 3.2). With the context stack, rules receive a `&NodeView` with everything already resolved. No futures, no Mutex, no channel await. Rules that need async (e.g., external lookups) can opt in via a separate trait method.

3. **Flatten the builder** (section 6.1). Replace recursive typestate generics with a flat declarative API. Resolve parent/children wiring at build time. Validate structure in `.build()`.

4. **Emit diagnostics per element, not per path** (section 3.3). The fold/assert split discards element identity. Instead, rules should produce `Vec<ElementDiagnostic>` with element index, position, and the specific failure. The fold accumulator becomes optional — used only for aggregate rules.

5. **Shift parallelism from paths to files** (section 5.1). With synchronous rules evaluated in the reader task, each file runs in its own task. File-level parallelism is natural (files are independent) and avoids all the channel topology complexity of cross-task rule execution.

The current architecture is inventive — streaming XML with async channels for cross-node communication is a novel approach. But the complexity-to-benefit ratio is unfavorable. The channel infrastructure dominates the runtime cost, and the relational access patterns it enables (parent/children) can be achieved more simply with a parse-time context stack. The streaming property (bounded memory) is preserved. The async property (non-blocking I/O) is preserved at the file reader level. What's removed is the async tax on rule evaluation, where it doesn't belong.