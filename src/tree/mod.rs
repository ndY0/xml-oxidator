//! Descriptor tree: declarative schema of expected XML structure with per-node access modes and rules.

/// Fluent builder API for constructing a [`DescriptorTree`](descriptor::DescriptorTree).
pub mod builder;
/// Core descriptor types: [`DescriptorNode`](descriptor::DescriptorNode),
/// [`DescriptorTree`](descriptor::DescriptorTree), and [`NodeNeeds`](descriptor::NodeNeeds).
pub mod descriptor;
/// Path types: [`PathSegment`](path::PathSegment), [`NodeId`](path::NodeId).
pub mod path;
