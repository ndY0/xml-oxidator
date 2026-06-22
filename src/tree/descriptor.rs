use std::collections::HashMap;

use crate::rule::Rule;
use crate::tree::path::{NodeId, PathSegment};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessMode {
    Streaming,
    CaptureSubtree,
}

bitflags! {
    /// What data a node's rules actually access, computed at build time.
    /// When a flag is absent, the parser skips collecting that data entirely.
    pub struct NodeNeeds: u8 {
        const ATTRS    = 0b0001;
        const TEXT     = 0b0010;
        const CHILDREN = 0b0100;
    }
}

pub struct DescriptorNode {
    pub tag: PathSegment,
    pub full_path: Vec<PathSegment>,
    pub access_mode: AccessMode,
    pub rules: Vec<Box<dyn Rule>>,
    pub parent_id: Option<NodeId>,
    pub child_ids: Vec<NodeId>,
    pub child_tag_index: HashMap<PathSegment, NodeId>,
    pub needs: NodeNeeds,
    /// What ancestors need from this node when it appears as a ChildSummary.
    /// Computed by OR-ing parent_needs of ancestor nodes.
    pub summary_needs: NodeNeeds,
}

pub struct DescriptorTree {
    pub(crate) nodes: Vec<DescriptorNode>,
    pub(crate) root_id: Option<NodeId>,
    pub capture_memory_limit: usize,
}

impl DescriptorTree {
    pub fn root(&self) -> Option<&DescriptorNode> {
        self.root_id.map(|id| &self.nodes[id.0])
    }

    pub fn root_id(&self) -> Option<NodeId> {
        self.root_id
    }

    pub fn get(&self, id: NodeId) -> &DescriptorNode {
        &self.nodes[id.0]
    }

    pub fn child_of(&self, parent_id: NodeId, tag: &str) -> Option<NodeId> {
        let parent = &self.nodes[parent_id.0];
        parent.child_tag_index.get(tag).copied()
    }
}

macro_rules! bitflags {
    (
        $(#[$outer:meta])*
        pub struct $Name:ident: $T:ty {
            $($(#[$inner:meta])* const $Flag:ident = $value:expr;)*
        }
    ) => {
        $(#[$outer])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub struct $Name($T);

        impl $Name {
            $($(#[$inner])* pub const $Flag: Self = Self($value);)*

            pub const fn empty() -> Self { Self(0) }
            pub const fn all() -> Self { Self($($value)|*) }
            pub const fn contains(self, other: Self) -> bool { (self.0 & other.0) == other.0 }
            pub const fn union(self, other: Self) -> Self { Self(self.0 | other.0) }
        }

        impl std::ops::BitOr for $Name {
            type Output = Self;
            fn bitor(self, rhs: Self) -> Self { Self(self.0 | rhs.0) }
        }

        impl std::ops::BitOrAssign for $Name {
            fn bitor_assign(&mut self, rhs: Self) { self.0 |= rhs.0; }
        }
    };
}

pub(crate) use bitflags;
