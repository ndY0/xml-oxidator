//! Streaming XML validation library with synchronous rule execution.
//!
//! Implements a hybrid streaming/capture model: XML is parsed via `quick-xml` in a single
//! forward pass, rules evaluate synchronously at element close time, and file-level
//! parallelism is achieved via rayon.
//!
//! # Architecture
//!
//! ```text
//! FileInfo (lazy stream factory)
//!   → rayon thread pool (one file per task)
//!     → parse_file(): context-stack streaming + selective subtree capture
//!       → Rules evaluate synchronously at </end> time
//!         → Diagnostics streamed via crossbeam channel
//! ```
//!
//! Each descriptor node declares an access mode via [`tree::descriptor::NodeNeeds`]:
//! - **Streaming** — O(depth) memory, context-stack with attrs/text/children summaries.
//! - **CaptureSubtree** — buffers XML events, materializes a mini-DOM at element close.

#[cfg(feature = "macros")]
mod macros;

/// Validation output types: diagnostics and severity levels.
pub mod diagnostic;
/// Error types for the builder, reader, and pipeline stages.
pub mod error;
/// File-parallel validation pipeline with rayon thread pool.
pub mod pipeline;
/// XML event-loop parser with streaming/capture modal processing.
pub mod reader;
/// Rule and node-access traits for synchronous validation.
pub mod rule;
/// Descriptor tree: declarative schema of expected XML structure.
pub mod tree;
/// View types that rules receive: streaming context and captured subtrees.
pub mod view;
