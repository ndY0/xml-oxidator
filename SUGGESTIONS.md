# Code Review Suggestions

## Architecture

The pipeline design (reader -> worker -> collector) with channel-based communication is solid for this kind of streaming workload. The typestate builder pattern in `rulebuilder.rs` (`NoTest -> NoFold -> NoInit -> NoAssert`) is a good use of the type system to enforce correct construction order at compile time.

However, the responsibilities between modules are tangled. `init.rs` contains `FatalError`, `ValidatorError`, `FileInfo`, and the `start` orchestrator — these are four distinct concerns. `rulebuilder.rs` at 688 lines holds the builder, tree data structures, node views, the `Rule` trait, `Path`, `Diagnostic`, and `RuleResult`. Consider splitting along these boundaries:
- Tree/Node/Path as a `tree.rs` module
- NodeView/PartialNodeView/FullNodeView as a `view.rs` module
- Rule/RuleBuilder/ConcreteRule as `rule.rs`
- Error types could live near the modules that produce them, or in a shared `error.rs`

## Idiomatic Rust Tips

### 1. Error handling — use `thiserror` instead of manual `Display` impls

You have 5 error types (`FatalError`, `ValidatorError`, `FileReaderError`, `ConsumerError`, `CollectorError`) all with identical boilerplate — a `String` field, a manual `Display` impl, and manual `From` impls. `thiserror` eliminates this:

```rust
#[derive(Debug, thiserror::Error)]
#[error("fatal error: {message}")]
pub struct FatalError { pub message: String }
```

### 2. Replace `match` on `bool`/`Option` with `if let`

Throughout the codebase you write patterns like (`filereader.rs:83-98`):

```rust
let matched = match tree.children(...).get(path) {
    Some(child) => { maybe_child = Some(child); true },
    None => false
};
match maybe_child { Some(child) => { ... }, None => {} };
```

This is just:
```rust
if let Some(child) = tree.children(...).get(path) {
    self.current_descriptor = child;
    return true;
}
false
```

Similarly, `match x { Some(v) => { ... }, None => {} }` throughout the code should be `if let Some(v) = x { ... }`. This appears in `filereader.rs:101-108`, `filereader.rs:344-375`, `filereader.rs:457-462`, `rulebuilder.rs:278-283`, and many other places.

### 3. Replace `match on Ok/Err` with `?` or combinators

You have dozens of instances of:
```rust
match sender.send(x).await {
    Ok(()) => {},
    Err(err) => { fatal_error_handle.trigger_fatal(err.into()).await; }
};
```

Consider extracting a helper method on `ShutdownHandle` that wraps a fallible operation, or at minimum use `if let Err(e) = ...` instead of the full match when you only care about the error branch.

### 4. `impl Display` for `Path` instead of `impl ToString`

`rulebuilder.rs:679-683` — implementing `ToString` directly is an anti-pattern. Implement `Display` instead; you get `ToString` for free via the blanket impl. The compiler even warns about this.

### 5. `impl From<X> for Y` instead of `impl Into<Y> for X`

`rulebuilder.rs:666-674` — `impl Into<PartialNodeView> for FullNodeView` should be `impl From<FullNodeView> for PartialNodeView`. The `From`/`Into` convention is always to implement `From`; the `Into` direction comes free.

### 6. Unnecessary `Arc` wrapping

- `xmlworker.rs:50` — `collector_sender: Arc<Sender<FileResult>>` — `Sender` is already cheaply cloneable (it's internally ref-counted). Wrapping it in `Arc` adds an extra indirection for no benefit.
- `filereader.rs:47` — `sender: Sender<Arc<FullNodeView>>` — sending `Arc<FullNodeView>` through a channel that's already bounded adds allocation overhead. If only one consumer reads each view, plain `FullNodeView` would work.

### 7. `Mutex` where none is needed

- `collector.rs:66` — `let results: Mutex<HashMap<...>> = Mutex::new(...)` is created locally and only used inside a single `loop` on a single task. No concurrent access exists, so the Mutex is pure overhead.
- `filereader.rs:202` — same pattern with `ReaderContext`. It's created and locked inside a single task.
- `init.rs:147` — `Arc<Mutex<Receiver>>` for sharing a receiver across readers is a code smell. Consider using a fan-out pattern (one dedicated dispatcher task that sends to per-reader channels) instead of contending on a shared receiver.

### 8. Avoid cloning where borrows suffice

- `rulebuilder.rs:299-304` — `Tree::children` returns `HashMap<Path, &Node>`, cloning every `Path` key on every call. Return `&HashMap<Path, usize>` (i.e., `node.nodes()`) and let callers look up by reference.
- `rulebuilder.rs:356` — `Node::rules()` clones every `Box<dyn Rule>` (which itself calls `clone_box`, heap-allocating). This is called per element match. If rules are read-only during processing, share them via `Arc<[Box<dyn Rule>]>` instead.
- `filereader.rs:270` — `Path(String::from_utf8_lossy(...).into())` followed by matching against `reader_context.current_descriptor.path().0.as_bytes()` — you're allocating a `String` just to compare bytes. Compare bytes directly first, construct the `Path` only when needed.

### 9. Use `RuleResult` as a proper struct, not a tuple struct

`rulebuilder.rs:362` — `pub struct RuleResult(pub String, pub String, pub bool, pub String)` — four positional fields with no names makes call sites like `RuleResult(path, name, status, msg)` order-dependent and fragile. Named fields would make this self-documenting.

### 10. Spelling / naming

- `statut` (`rulebuilder.rs:569,578`) — should be `status`
- `diagnotics` (`collector.rs:33`) — should be `diagnostics`
- `depeer` in test name — should be `deeper`
- `occured` — should be `occurred` (appears in multiple error messages)
- `accomodate` — should be `accommodate`
- `apppeared` — should be `appeared`

### 11. `collect_results` return type

`collector.rs:65` — `-> ()` is redundant; Rust functions return unit by default. Just omit it.

### 12. `FileResult` enum variants carry too many positional args

`FileResult::Progress(u64, String, u8, Vec<RuleResult>)` — four unnamed fields. This is the same readability problem as `RuleResult`. Use named fields:

```rust
enum FileResult {
    Progress { file_id: u64, file: String, counter: u8, results: Vec<RuleResult> },
    // ...
}
```

### 13. `collector.rs` has massive duplication

The four `match file_result` arms in `process_file_result` (`collector.rs:106-250`) repeat nearly identical logic — insert-or-update a map entry, optionally push rule results, check completion, conditionally remove. This could be collapsed using the `Entry` API (`results.entry(file_id).or_insert_with(...)`) plus a shared post-processing step.

### 14. Tests silently swallow errors

In `integration_test.rs`, every `match` on `Ok`/`Err` has empty bodies for both branches. A failing `start` or `send` would pass silently. Use `.unwrap()` or `.expect()` in tests — they should panic on unexpected failures.
