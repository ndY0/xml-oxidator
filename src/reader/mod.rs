//! XML event-loop parser with modal streaming/capture processing.

/// Memory-limited subtree capture buffer and arena materialization.
pub(crate) mod capture;
/// Per-element context stack frames and object pools for allocation reuse.
pub mod context;
/// Core `parse_file` / `parse_slice` event loops with rule evaluation.
pub mod parser;
