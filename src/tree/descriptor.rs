use crate::tree::path::{NodeId, PathSegment};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NodeNeeds(u8);

impl NodeNeeds {
    pub const ATTRS: Self = Self(0x01);
    pub const TEXT: Self = Self(0x02);
    pub const CHILDREN: Self = Self(0x04);
    pub const CAPTURE: Self = Self(0x08);

    #[inline(always)]
    pub const fn empty() -> Self { Self(0) }

    /// ATTRS | TEXT | CHILDREN (not CAPTURE)
    #[inline(always)]
    pub const fn all() -> Self { Self(0x07) }

    #[inline(always)]
    pub const fn contains(self, other: Self) -> bool { (self.0 & other.0) == other.0 }

    #[inline(always)]
    pub const fn is_capture(self) -> bool { self.0 & 0x08 != 0 }

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

pub struct DescriptorNode<R> {
    // Pointers / Vecs first (8-byte aligned)
    pub full_path: Vec<PathSegment>,
    pub rules: Vec<R>,
    pub children_sorted: Vec<(PathSegment, NodeId)>,
    pub child_ids: Vec<NodeId>,
    pub tag: PathSegment,
    // Then Option<NodeId> (u32 + discriminant)
    pub parent_id: Option<NodeId>,
    // Then u8s
    pub needs: NodeNeeds,
    pub summary_needs: NodeNeeds,
}

pub struct DescriptorTree<R> {
    pub(crate) nodes: Vec<DescriptorNode<R>>,
    pub(crate) root_id: Option<NodeId>,
    pub capture_memory_limit: usize,
}

impl<R> DescriptorTree<R> {
    #[inline(always)]
    pub fn root(&self) -> Option<&DescriptorNode<R>> {
        self.root_id.map(|id| &self.nodes[id.0 as usize])
    }

    #[inline(always)]
    pub fn root_id(&self) -> Option<NodeId> {
        self.root_id
    }

    #[inline(always)]
    pub fn get(&self, id: NodeId) -> &DescriptorNode<R> {
        &self.nodes[id.0 as usize]
    }

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
