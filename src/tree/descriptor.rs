use crate::tree::path::{NodeId, PathSegment};

/// Bitflags declaring what data a node or rule needs from an XML element.
///
/// Used by the parser to skip unnecessary work (e.g. not capturing attrs
/// when no rule reads them).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NodeNeeds(u8);

impl NodeNeeds {
    /// The rule reads element attributes.
    pub const ATTRS: Self = Self(0x01);
    /// The rule reads element text content.
    pub const TEXT: Self = Self(0x02);
    /// The rule reads children summaries.
    pub const CHILDREN: Self = Self(0x04);
    /// The rule requires full subtree capture (implies `CaptureSubtree` access mode).
    pub const CAPTURE: Self = Self(0x08);

    /// No data needed.
    #[inline(always)]
    pub const fn empty() -> Self { Self(0) }

    /// ATTRS | TEXT | CHILDREN (not CAPTURE).
    #[inline(always)]
    pub const fn all() -> Self { Self(0x07) }

    /// Returns `true` if all bits in `other` are set in `self`.
    #[inline(always)]
    pub const fn contains(self, other: Self) -> bool { (self.0 & other.0) == other.0 }

    /// Returns `true` if the CAPTURE bit is set.
    #[inline(always)]
    pub const fn is_capture(self) -> bool { self.0 & 0x08 != 0 }

    /// Returns the bitwise union of `self` and `other`.
    #[inline(always)]
    pub const fn union(self, other: Self) -> Self { Self(self.0 | other.0) }
}

impl std::ops::BitOr for NodeNeeds {
    type Output = Self;
    #[inline(always)]
    fn bitor(self, rhs: Self) -> Self { Self(self.0 | rhs.0) }
}

impl std::ops::BitOrAssign for NodeNeeds {
    #[inline(always)]
    fn bitor_assign(&mut self, rhs: Self) { self.0 |= rhs.0; }
}

/// A single node in the descriptor tree, representing an expected XML element.
///
/// Holds the tag name, attached rules, child relationships, and data-need flags
/// that control streaming vs. capture behavior during parsing.
pub struct DescriptorNode<R> {
    /// Full path from root to this node (e.g. `["catalog", "entry", "detail"]`).
    pub full_path: Vec<PathSegment>,
    /// Validation rules attached to this node, evaluated at element close.
    pub rules: Vec<R>,
    /// Children sorted by tag name for O(log n) binary-search lookup.
    pub children_sorted: Vec<(PathSegment, NodeId)>,
    /// Children in declaration order.
    pub child_ids: Vec<NodeId>,
    /// Tag name of this element.
    pub tag: PathSegment,
    /// Parent node ID, or `None` for the root.
    pub parent_id: Option<NodeId>,
    /// Bitflags of what data this node's rules need.
    pub needs: NodeNeeds,
    /// Bitflags of what data the parent needs from this node's child summary.
    pub summary_needs: NodeNeeds,
}

/// A complete descriptor tree defining the expected XML structure, access modes, and rules.
///
/// Built via [`TreeBuilder`](super::builder::TreeBuilder). Shared across files via `Arc`.
pub struct DescriptorTree<R> {
    pub(crate) nodes: Vec<DescriptorNode<R>>,
    pub(crate) root_id: Option<NodeId>,
    /// Maximum bytes a single subtree capture may consume before erroring.
    pub capture_memory_limit: usize,
}

impl<R> DescriptorTree<R> {
    /// Returns the root descriptor node, if the tree has one.
    #[inline(always)]
    pub fn root(&self) -> Option<&DescriptorNode<R>> {
        self.root_id.map(|id| &self.nodes[id.0 as usize])
    }

    /// Returns the root node's ID, if the tree has one.
    #[inline(always)]
    pub fn root_id(&self) -> Option<NodeId> {
        self.root_id
    }

    /// Returns the descriptor node for the given ID (panics if out of bounds).
    #[inline(always)]
    pub fn get(&self, id: NodeId) -> &DescriptorNode<R> {
        &self.nodes[id.0 as usize]
    }

    /// Looks up a child descriptor by tag name under the given parent (O(log n) binary search).
    #[inline(always)]
    pub fn child_of(&self, parent_id: NodeId, tag: &str) -> Option<NodeId> {
        let parent = self.get(parent_id);
        parent
            .children_sorted
            .binary_search_by(|(k, _)| k.0.as_ref().cmp(tag))
            .ok()
            .map(|i| parent.children_sorted[i].1)
    }
}
