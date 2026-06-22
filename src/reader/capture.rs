use bumpalo::Bump;

use crate::error::ReaderError;
use crate::view::SubtreeNode;

struct BuildNode {
    tag: String,
    attrs: Vec<(String, String)>,
    text: String,
    children: Vec<BuildNode>,
}

pub(crate) struct CaptureBuilder {
    stack: Vec<BuildNode>,
    memory_usage: usize,
    memory_limit: usize,
    path_for_error: String,
}

impl CaptureBuilder {
    #[inline]
    pub fn new(limit: usize, path_for_error: String) -> Self {
        Self {
            stack: Vec::new(),
            memory_usage: 0,
            memory_limit: limit,
            path_for_error,
        }
    }

    #[inline]
    pub fn start_element(
        &mut self,
        tag: &str,
        attrs: Vec<(String, String)>,
    ) -> Result<(), ReaderError> {
        let size = size_of::<BuildNode>()
            + tag.len()
            + attrs.iter().map(|(k, v)| k.len() + v.len() + 64).sum::<usize>();
        self.memory_usage += size;
        self.check_limit()?;
        self.stack.push(BuildNode {
            tag: tag.to_owned(),
            attrs,
            text: String::new(),
            children: Vec::new(),
        });
        Ok(())
    }

    #[inline]
    pub fn end_element(&mut self) {
        let node = self.stack.pop().expect("unbalanced capture events");
        if let Some(parent) = self.stack.last_mut() {
            parent.children.push(node);
        } else {
            self.stack.push(node);
        }
    }

    #[inline]
    pub fn text(&mut self, text: &str) -> Result<(), ReaderError> {
        if text.is_empty() {
            return Ok(());
        }
        self.memory_usage += text.len();
        self.check_limit()?;
        if let Some(top) = self.stack.last_mut() {
            if top.text.is_empty() {
                top.text = text.to_owned();
            } else {
                top.text.push_str(text);
            }
        }
        Ok(())
    }

    #[inline]
    pub fn finalize<'a>(self, arena: &'a Bump) -> &'a SubtreeNode<'a> {
        let root = self.stack.into_iter().next().expect("empty capture");
        Self::build_arena_node(root, arena)
    }

    fn build_arena_node<'a>(node: BuildNode, arena: &'a Bump) -> &'a SubtreeNode<'a> {
        let tag = arena.alloc_str(&node.tag);
        let text = arena.alloc_str(&node.text);

        let attrs_slice = {
            let mut attrs = bumpalo::collections::Vec::with_capacity_in(node.attrs.len(), arena);
            for (k, v) in &node.attrs {
                let k: &'a str = arena.alloc_str(k);
                let v: &'a str = arena.alloc_str(v);
                attrs.push((k, v));
            }
            attrs.into_bump_slice()
        };

        let children_slice = {
            let mut children =
                bumpalo::collections::Vec::with_capacity_in(node.children.len(), arena);
            for child in node.children {
                let child_ref = Self::build_arena_node(child, arena);
                children.push(SubtreeNode {
                    tag: child_ref.tag,
                    attrs: child_ref.attrs,
                    text: child_ref.text,
                    children: child_ref.children,
                });
            }
            children.into_bump_slice()
        };

        arena.alloc(SubtreeNode {
            tag,
            attrs: attrs_slice,
            text,
            children: children_slice,
        })
    }

    #[inline]
    fn check_limit(&self) -> Result<(), ReaderError> {
        if self.memory_usage > self.memory_limit {
            Err(ReaderError::CaptureOverflow {
                path: self.path_for_error.clone(),
                limit: self.memory_limit,
            })
        } else {
            Ok(())
        }
    }
}
