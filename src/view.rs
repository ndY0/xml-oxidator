use crate::reader::context::NodeContext;
use crate::rule::NodeAccess;
use crate::tree::descriptor::DescriptorTree;
use crate::tree::path::{NodeId, PathSegment};

/// Bitpacked pass/fail results for up to 64 rules on a single element.
#[derive(Debug, Clone, Copy)]
pub struct RuleResults(pub u64);

impl RuleResults {
    /// Creates a result set with all rules failed (no bits set).
    #[inline(always)]
    pub const fn empty() -> Self { Self(0) }

    /// Sets the result for the rule at `index` (0-based).
    #[inline(always)]
    pub fn set(&mut self, index: usize, passed: bool) {
        if passed { self.0 |= 1 << index; }
    }

    /// Returns whether the rule at `index` passed.
    #[inline(always)]
    pub fn get(self, index: usize) -> bool {
        (self.0 >> index) & 1 == 1
    }

    /// Returns `true` if all `count` rules passed.
    #[inline(always)]
    pub fn all_passed(self, count: usize) -> bool {
        let mask = if count >= 64 { u64::MAX } else { (1u64 << count) - 1 };
        (self.0 & mask) == mask
    }
}

/// Summary of a completed child element, pushed to its parent's context at `</end>`.
///
/// Contains the child's attributes, text, descriptor identity, sibling index,
/// and bitpacked rule results. Visible to the parent via [`NodeAccess::children_summaries`].
#[derive(Debug, Clone)]
pub struct ChildSummary {
    /// Attributes of the child element.
    pub attrs: Vec<(String, String)>,
    /// Direct text content of the child element.
    pub text: String,
    /// Descriptor node ID identifying the child's position in the descriptor tree.
    pub descriptor_id: NodeId,
    /// Zero-based sibling index among same-tag siblings.
    pub index: u32,
    /// Bitpacked pass/fail results of all rules evaluated on this child.
    pub rule_results: RuleResults,
}

impl ChildSummary {
    /// Looks up a child attribute by name.
    #[inline]
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    /// Returns the tag name of this child by looking it up in the descriptor tree.
    #[inline]
    pub fn tag<'a, R>(&self, tree: &'a DescriptorTree<R>) -> &'a str {
        tree.get(self.descriptor_id).tag.0.as_ref()
    }
}

/// A node in a bump-allocated mini-DOM materialized from a captured subtree.
///
/// Provides DOM-like query methods ([`find`](SubtreeNode::find),
/// [`find_all`](SubtreeNode::find_all), [`descendants`](SubtreeNode::descendants))
/// for rules that need full subtree access.
pub struct SubtreeNode<'a> {
    /// Tag name of this element.
    pub tag: &'a str,
    /// Attributes as (key, value) pairs.
    pub attrs: &'a [(&'a str, &'a str)],
    /// Direct text content.
    pub text: &'a str,
    /// Immediate child nodes.
    pub children: &'a [SubtreeNode<'a>],
}

impl<'a> SubtreeNode<'a> {
    /// Returns a depth-first iterator over all descendant nodes.
    #[inline]
    pub fn descendants(&self) -> SubtreeDescendants<'a, '_> {
        SubtreeDescendants {
            stack: self.children.iter().rev().collect(),
        }
    }

    /// Finds the first descendant with the given tag name (depth-first).
    #[inline]
    pub fn find(&self, tag: &str) -> Option<&SubtreeNode<'a>> {
        self.descendants().find(|n| n.tag == tag)
    }

    /// Finds all descendants with the given tag name (depth-first).
    #[inline]
    pub fn find_all(&self, tag: &str) -> Vec<&SubtreeNode<'a>> {
        self.descendants().filter(|n| n.tag == tag).collect()
    }

    /// Looks up an attribute by name on this subtree node.
    #[inline]
    pub fn attr(&self, name: &str) -> Option<&'a str> {
        self.attrs.iter().find(|(k, _)| *k == name).map(|(_, v)| *v)
    }
}

/// Depth-first iterator over all descendants of a [`SubtreeNode`].
pub struct SubtreeDescendants<'a, 'b> {
    stack: Vec<&'b SubtreeNode<'a>>,
}

impl<'a, 'b> Iterator for SubtreeDescendants<'a, 'b> {
    type Item = &'b SubtreeNode<'a>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let node = self.stack.pop()?;
        self.stack.extend(node.children.iter().rev());
        Some(node)
    }
}

/// A streaming-mode view of an XML element backed by the context stack.
///
/// Provides O(1) parent access via stack indexing and exposes completed
/// children summaries accumulated during parsing.
pub struct StreamingView<'a, R> {
    pub(crate) stack: &'a [NodeContext],
    pub(crate) index_in_stack: usize,
    pub(crate) tree: &'a DescriptorTree<R>,
}

impl<'a, R> StreamingView<'a, R> {
    /// Returns a view of the parent element, or `None` if this is the root.
    #[inline]
    pub fn parent(&self) -> Option<StreamingView<'a, R>> {
        if self.index_in_stack == 0 {
            None
        } else {
            Some(StreamingView {
                stack: self.stack,
                index_in_stack: self.index_in_stack - 1,
                tree: self.tree,
            })
        }
    }

    #[inline(always)]
    fn ctx(&self) -> &NodeContext {
        &self.stack[self.index_in_stack]
    }
}

impl<R> NodeAccess for StreamingView<'_, R> {
    #[inline]
    fn tag(&self) -> &str {
        self.tree.get(self.ctx().descriptor_id).tag.0.as_ref()
    }

    #[inline]
    fn attr(&self, name: &str) -> Option<&str> {
        self.ctx().attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    #[inline]
    fn for_each_attr(&self, f: &mut dyn FnMut(&str, &str)) {
        for (k, v) in &self.ctx().attrs {
            f(k, v);
        }
    }

    #[inline]
    fn text(&self) -> &str {
        &self.ctx().text
    }

    #[inline]
    fn element_index(&self) -> usize {
        self.ctx().index as usize
    }

    #[inline]
    fn path(&self) -> &[PathSegment] {
        &self.tree.get(self.ctx().descriptor_id).full_path
    }

    #[inline]
    fn children_summaries(&self) -> &[ChildSummary] {
        &self.ctx().children
    }

    #[inline]
    fn subtree(&self) -> Option<&SubtreeNode<'_>> {
        None
    }

    #[inline]
    fn depth(&self) -> usize {
        self.index_in_stack
    }

    #[inline]
    fn ancestor_attr(&self, level: usize, name: &str) -> Option<&str> {
        let idx = self.index_in_stack.checked_sub(level + 1)?;
        self.stack[idx].attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    #[inline]
    fn ancestor_text(&self, level: usize) -> Option<&str> {
        let idx = self.index_in_stack.checked_sub(level + 1)?;
        Some(&self.stack[idx].text)
    }

    #[inline]
    fn ancestor_children(&self, level: usize) -> Option<&[ChildSummary]> {
        let idx = self.index_in_stack.checked_sub(level + 1)?;
        Some(&self.stack[idx].children)
    }
}

/// A capture-mode view that wraps a materialized [`SubtreeNode`] mini-DOM.
///
/// Provides full DOM query access to the captured subtree while still
/// allowing ancestor access via the parent context stack.
pub struct SubtreeView<'a, R> {
    pub(crate) root: &'a SubtreeNode<'a>,
    pub(crate) descriptor_id: NodeId,
    pub(crate) index: u32,
    pub(crate) parent_stack: &'a [NodeContext],
    pub(crate) tree: &'a DescriptorTree<R>,
}

impl<R> NodeAccess for SubtreeView<'_, R> {
    #[inline]
    fn tag(&self) -> &str {
        self.root.tag
    }

    #[inline]
    fn attr(&self, name: &str) -> Option<&str> {
        self.root.attr(name)
    }

    #[inline]
    fn for_each_attr(&self, f: &mut dyn FnMut(&str, &str)) {
        for &(k, v) in self.root.attrs {
            f(k, v);
        }
    }

    #[inline]
    fn text(&self) -> &str {
        self.root.text
    }

    #[inline]
    fn element_index(&self) -> usize {
        self.index as usize
    }

    #[inline]
    fn path(&self) -> &[PathSegment] {
        &self.tree.get(self.descriptor_id).full_path
    }

    #[inline]
    fn children_summaries(&self) -> &[ChildSummary] {
        &[]
    }

    #[inline]
    fn subtree(&self) -> Option<&SubtreeNode<'_>> {
        Some(self.root)
    }

    #[inline]
    fn depth(&self) -> usize {
        self.parent_stack.len()
    }

    #[inline]
    fn ancestor_attr(&self, level: usize, name: &str) -> Option<&str> {
        let idx = self.parent_stack.len().checked_sub(level + 1)?;
        self.parent_stack[idx].attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    #[inline]
    fn ancestor_text(&self, level: usize) -> Option<&str> {
        let idx = self.parent_stack.len().checked_sub(level + 1)?;
        Some(&self.parent_stack[idx].text)
    }

    #[inline]
    fn ancestor_children(&self, level: usize) -> Option<&[ChildSummary]> {
        let idx = self.parent_stack.len().checked_sub(level + 1)?;
        Some(&self.parent_stack[idx].children)
    }
}
