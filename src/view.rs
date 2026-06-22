use smallvec::SmallVec;

use crate::reader::context::NodeContext;
use crate::rule::NodeAccess;
use crate::tree::descriptor::DescriptorTree;
use crate::tree::path::PathSegment;

#[derive(Debug, Clone)]
pub struct RuleResult {
    pub passed: bool,
}

#[derive(Debug, Clone)]
pub struct ChildSummary {
    pub tag: PathSegment,
    pub attrs: Vec<(String, String)>,
    pub text: String,
    pub index: usize,
    pub rule_results: SmallVec<[RuleResult; 4]>,
}

impl ChildSummary {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
}

pub struct SubtreeNode<'a> {
    pub tag: &'a str,
    pub attrs: &'a [(&'a str, &'a str)],
    pub text: &'a str,
    pub children: &'a [SubtreeNode<'a>],
}

impl<'a> SubtreeNode<'a> {
    pub fn descendants(&self) -> SubtreeDescendants<'a, '_> {
        SubtreeDescendants {
            stack: self.children.iter().rev().collect(),
        }
    }

    pub fn find(&self, tag: &str) -> Option<&SubtreeNode<'a>> {
        self.descendants().find(|n| n.tag == tag)
    }

    pub fn find_all(&self, tag: &str) -> Vec<&SubtreeNode<'a>> {
        self.descendants().filter(|n| n.tag == tag).collect()
    }

    pub fn attr(&self, name: &str) -> Option<&'a str> {
        self.attrs.iter().find(|(k, _)| *k == name).map(|(_, v)| *v)
    }
}

pub struct SubtreeDescendants<'a, 'b> {
    stack: Vec<&'b SubtreeNode<'a>>,
}

impl<'a, 'b> Iterator for SubtreeDescendants<'a, 'b> {
    type Item = &'b SubtreeNode<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let node = self.stack.pop()?;
        self.stack.extend(node.children.iter().rev());
        Some(node)
    }
}

pub struct StreamingView<'a> {
    pub(crate) stack: &'a [NodeContext],
    pub(crate) index_in_stack: usize,
    pub(crate) tree: &'a DescriptorTree,
}

impl<'a> StreamingView<'a> {
    pub fn parent(&self) -> Option<StreamingView<'a>> {
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

    fn ctx(&self) -> &NodeContext {
        &self.stack[self.index_in_stack]
    }
}

impl NodeAccess for StreamingView<'_> {
    fn tag(&self) -> &str {
        self.tree.get(self.ctx().descriptor_id).tag.0.as_ref()
    }

    fn attr(&self, name: &str) -> Option<&str> {
        self.ctx().attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    fn for_each_attr(&self, f: &mut dyn FnMut(&str, &str)) {
        for (k, v) in &self.ctx().attrs {
            f(k, v);
        }
    }

    fn text(&self) -> &str {
        &self.ctx().text
    }

    fn element_index(&self) -> usize {
        self.ctx().index
    }

    fn path(&self) -> &[PathSegment] {
        &self.tree.get(self.ctx().descriptor_id).full_path
    }

    fn children_summaries(&self) -> &[ChildSummary] {
        &self.ctx().children
    }

    fn subtree(&self) -> Option<&SubtreeNode<'_>> {
        None
    }

    fn depth(&self) -> usize {
        self.index_in_stack
    }

    fn ancestor_attr(&self, level: usize, name: &str) -> Option<&str> {
        let idx = self.index_in_stack.checked_sub(level + 1)?;
        self.stack[idx].attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    fn ancestor_text(&self, level: usize) -> Option<&str> {
        let idx = self.index_in_stack.checked_sub(level + 1)?;
        Some(&self.stack[idx].text)
    }

    fn ancestor_children(&self, level: usize) -> Option<&[ChildSummary]> {
        let idx = self.index_in_stack.checked_sub(level + 1)?;
        Some(&self.stack[idx].children)
    }
}

pub struct SubtreeView<'a> {
    pub(crate) root: &'a SubtreeNode<'a>,
    pub(crate) descriptor_id: crate::tree::path::NodeId,
    pub(crate) index: usize,
    pub(crate) parent_stack: &'a [NodeContext],
    pub(crate) tree: &'a DescriptorTree,
}

impl<'a> NodeAccess for SubtreeView<'a> {
    fn tag(&self) -> &str {
        self.root.tag
    }

    fn attr(&self, name: &str) -> Option<&str> {
        self.root.attr(name)
    }

    fn for_each_attr(&self, f: &mut dyn FnMut(&str, &str)) {
        for &(k, v) in self.root.attrs {
            f(k, v);
        }
    }

    fn text(&self) -> &str {
        self.root.text
    }

    fn element_index(&self) -> usize {
        self.index
    }

    fn path(&self) -> &[PathSegment] {
        &self.tree.get(self.descriptor_id).full_path
    }

    fn children_summaries(&self) -> &[ChildSummary] {
        &[]
    }

    fn subtree(&self) -> Option<&SubtreeNode<'_>> {
        Some(self.root)
    }

    fn depth(&self) -> usize {
        self.parent_stack.len()
    }

    fn ancestor_attr(&self, level: usize, name: &str) -> Option<&str> {
        let idx = self.parent_stack.len().checked_sub(level + 1)?;
        self.parent_stack[idx].attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    fn ancestor_text(&self, level: usize) -> Option<&str> {
        let idx = self.parent_stack.len().checked_sub(level + 1)?;
        Some(&self.parent_stack[idx].text)
    }

    fn ancestor_children(&self, level: usize) -> Option<&[ChildSummary]> {
        let idx = self.parent_stack.len().checked_sub(level + 1)?;
        Some(&self.parent_stack[idx].children)
    }
}
