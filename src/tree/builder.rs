use std::collections::HashMap;

use crate::error::BuilderError;
use crate::rule::Rule;
use crate::tree::descriptor::{AccessMode, DescriptorNode, DescriptorTree, NodeNeeds};
use crate::tree::path::{NodeId, PathSegment, format_path};

struct NodeDeclaration {
    tag: PathSegment,
    full_path: Vec<PathSegment>,
    access_mode: AccessMode,
    rules: Vec<Box<dyn Rule>>,
    parent_index: Option<usize>,
}

pub struct TreeBuilder {
    declarations: Vec<NodeDeclaration>,
    parent_stack: Vec<usize>,
    capture_memory_limit: usize,
}

impl TreeBuilder {
    pub fn new(root_tag: &str) -> Self {
        let mut builder = TreeBuilder {
            declarations: Vec::new(),
            parent_stack: Vec::new(),
            capture_memory_limit: 64 * 1024 * 1024,
        };
        builder.declarations.push(NodeDeclaration {
            tag: PathSegment::from(root_tag),
            full_path: vec![PathSegment::from(root_tag)],
            access_mode: AccessMode::Streaming,
            rules: Vec::new(),
            parent_index: None,
        });
        builder.parent_stack.push(0);
        builder
    }

    pub fn streaming(mut self) -> Self {
        let idx = *self.parent_stack.last().unwrap();
        self.declarations[idx].access_mode = AccessMode::Streaming;
        self
    }

    pub fn capture_subtree(mut self) -> Self {
        let idx = *self.parent_stack.last().unwrap();
        self.declarations[idx].access_mode = AccessMode::CaptureSubtree;
        self
    }

    pub fn rule(mut self, rule: Box<dyn Rule>) -> Self {
        let idx = *self.parent_stack.last().unwrap();
        self.declarations[idx].rules.push(rule);
        self
    }

    pub fn node(mut self, tag: &str) -> Self {
        let parent_idx = *self.parent_stack.last().unwrap();
        let mut full_path = self.declarations[parent_idx].full_path.clone();
        full_path.push(PathSegment::from(tag));
        let child_idx = self.declarations.len();
        self.declarations.push(NodeDeclaration {
            tag: PathSegment::from(tag),
            full_path,
            access_mode: AccessMode::Streaming,
            rules: Vec::new(),
            parent_index: Some(parent_idx),
        });
        self.parent_stack.push(child_idx);
        self
    }

    pub fn done(mut self) -> Self {
        assert!(
            self.parent_stack.len() > 1,
            "done() called on root node"
        );
        self.parent_stack.pop();
        self
    }

    pub fn capture_limit(mut self, bytes: usize) -> Self {
        self.capture_memory_limit = bytes;
        self
    }

    pub fn build(self) -> Result<DescriptorTree, BuilderError> {
        if self.declarations.is_empty() {
            return Err(BuilderError::NoRoot);
        }

        let path_to_index: HashMap<Vec<PathSegment>, usize> = self
            .declarations
            .iter()
            .enumerate()
            .map(|(i, d)| (d.full_path.clone(), i))
            .collect();

        if path_to_index.len() != self.declarations.len() {
            for (i, d) in self.declarations.iter().enumerate() {
                for (j, d2) in self.declarations.iter().enumerate() {
                    if i != j && d.full_path == d2.full_path {
                        return Err(BuilderError::DuplicatePath {
                            path: format_path(&d.full_path),
                        });
                    }
                }
            }
        }

        for decl in &self.declarations {
            for rule in &decl.rules {
                if rule.access_mode() == AccessMode::CaptureSubtree
                    && decl.access_mode == AccessMode::Streaming
                {
                    return Err(BuilderError::IncompatibleAccessMode {
                        node_path: format_path(&decl.full_path),
                        rule_name: rule.name().to_owned(),
                    });
                }
            }
        }

        for decl in &self.declarations {
            if decl.access_mode == AccessMode::CaptureSubtree {
                let mut ancestor_idx = decl.parent_index;
                while let Some(idx) = ancestor_idx {
                    if self.declarations[idx].access_mode == AccessMode::CaptureSubtree {
                        return Err(BuilderError::NestedCapture {
                            inner: format_path(&decl.full_path),
                            outer: format_path(&self.declarations[idx].full_path),
                        });
                    }
                    ancestor_idx = self.declarations[idx].parent_index;
                }
            }
        }

        let mut nodes: Vec<DescriptorNode> = Vec::with_capacity(self.declarations.len());
        for decl in self.declarations {
            let needs = decl.rules.iter().fold(NodeNeeds::empty(), |acc, r| acc | r.needs());
            nodes.push(DescriptorNode {
                tag: decl.tag,
                full_path: decl.full_path,
                access_mode: decl.access_mode,
                rules: decl.rules,
                parent_id: decl.parent_index.map(NodeId),
                child_ids: Vec::new(),
                child_tag_index: HashMap::new(),
                needs,
                summary_needs: NodeNeeds::empty(),
            });
        }

        for i in 0..nodes.len() {
            if let Some(parent_id) = nodes[i].parent_id {
                let child_id = NodeId(i);
                let child_tag = nodes[i].tag.clone();
                nodes[parent_id.0].child_ids.push(child_id);
                nodes[parent_id.0].child_tag_index.insert(child_tag, child_id);
            }
        }

        // Propagate needs upward: if any child's rules use ancestor_attr/ancestor_text,
        // the ancestors must store that data. Conservative: if a node has declared children
        // in the tree, keep attrs and text so descendants can access them.
        for node in &mut nodes {
            if !node.child_ids.is_empty() {
                node.needs = node.needs | NodeNeeds::ATTRS | NodeNeeds::TEXT;
            }
        }

        // Compute summary_needs: what does the parent need from this child?
        for i in 0..nodes.len() {
            if let Some(parent_id) = nodes[i].parent_id {
                let parent_needs = nodes[parent_id.0].needs;
                if parent_needs.contains(NodeNeeds::CHILDREN) {
                    nodes[i].summary_needs = NodeNeeds::all();
                }
            }
        }

        Ok(DescriptorTree {
            root_id: Some(NodeId(0)),
            nodes,
            capture_memory_limit: self.capture_memory_limit,
        })
    }
}
