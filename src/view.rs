use crate::reader::context::NodeContext;
use crate::rule::NodeAccess;
use crate::tree::descriptor::DescriptorTree;
use crate::tree::path::{NodeId, PathSegment};

#[derive(Debug, Clone, Copy)]
pub struct RuleResults(pub u64);

impl RuleResults {
    #[inline(always)]
    pub const fn empty() -> Self { Self(0) }

    #[inline(always)]
    pub fn set(&mut self, index: usize, passed: bool) {
        if passed { self.0 |= 1 << index; }
    }

    #[inline(always)]
    pub fn get(self, index: usize) -> bool {
        (self.0 >> index) & 1 == 1
    }

    #[inline(always)]
    pub fn all_passed(self, count: usize) -> bool {
        let mask = if count >= 64 { u64::MAX } else { (1u64 << count) - 1 };
        (self.0 & mask) == mask
    }
}

#[derive(Debug, Clone)]
pub struct ChildSummary {
    // Heap-allocated fields first
    pub attrs: Vec<(String, String)>,
    pub text: String,
    // Then smaller fields
    pub descriptor_id: NodeId,
    pub index: u32,
    pub rule_results: RuleResults,
}

impl ChildSummary {
    #[inline]
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    #[inline]
    pub fn tag<'a, R>(&self, tree: &'a DescriptorTree<R>) -> &'a str {
        tree.get(self.descriptor_id).tag.0.as_ref()
    }
}

pub struct SubtreeNode<'a> {
    pub tag: &'a str,
    pub attrs: &'a [(&'a str, &'a str)],
    pub text: &'a str,
    pub children: &'a [SubtreeNode<'a>],
}

impl<'a> SubtreeNode<'a> {
    #[inline]
    pub fn descendants(&self) -> SubtreeDescendants<'a, '_> {
        SubtreeDescendants {
            stack: self.children.iter().rev().collect(),
        }
    }

    #[inline]
    pub fn find(&self, tag: &str) -> Option<&SubtreeNode<'a>> {
        self.descendants().find(|n| n.tag == tag)
    }

    #[inline]
    pub fn find_all(&self, tag: &str) -> Vec<&SubtreeNode<'a>> {
        self.descendants().filter(|n| n.tag == tag).collect()
    }

    #[inline]
    pub fn attr(&self, name: &str) -> Option<&'a str> {
        self.attrs.iter().find(|(k, _)| *k == name).map(|(_, v)| *v)
    }
}

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

pub struct StreamingView<'a, R> {
    pub(crate) stack: &'a [NodeContext],
    pub(crate) index_in_stack: usize,
    pub(crate) tree: &'a DescriptorTree<R>,
}

impl<'a, R> StreamingView<'a, R> {
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
