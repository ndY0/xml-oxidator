use crate::error::BuilderError;
use crate::rule::Rule;
use crate::tree::descriptor::{DescriptorNode, DescriptorTree, NodeNeeds};
use crate::tree::path::{NodeId, PathSegment, format_path};

struct NodeDeclaration<R> {
    tag: PathSegment,
    full_path: Vec<PathSegment>,
    is_capture: bool,
    rules: Vec<R>,
    parent_index: Option<usize>,
}

pub struct TreeBuilder<R> {
    declarations: Vec<NodeDeclaration<R>>,
    parent_stack: Vec<usize>,
    capture_memory_limit: usize,
}

impl<R: Rule> TreeBuilder<R> {
    pub fn new(root_tag: &str) -> Self {
        let mut builder = TreeBuilder {
            declarations: Vec::new(),
            parent_stack: Vec::new(),
            capture_memory_limit: 64 * 1024 * 1024,
        };
        builder.declarations.push(NodeDeclaration {
            tag: PathSegment::from(root_tag),
            full_path: vec![PathSegment::from(root_tag)],
            is_capture: false,
            rules: Vec::new(),
            parent_index: None,
        });
        builder.parent_stack.push(0);
        builder
    }

    pub fn streaming(mut self) -> Self {
        let idx = *self.parent_stack.last().unwrap();
        self.declarations[idx].is_capture = false;
        self
    }

    pub fn capture_subtree(mut self) -> Self {
        let idx = *self.parent_stack.last().unwrap();
        self.declarations[idx].is_capture = true;
        self
    }

    pub fn rule(mut self, rule: R) -> Self {
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
            is_capture: false,
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

    pub fn build(self) -> Result<DescriptorTree<R>, BuilderError> {
        if self.declarations.is_empty() {
            return Err(BuilderError::NoRoot);
        }

        // Check for duplicate paths
        {
            let mut seen = std::collections::HashMap::with_capacity(self.declarations.len());
            for (i, d) in self.declarations.iter().enumerate() {
                if let Some(prev) = seen.insert(&d.full_path, i) {
                    let _ = prev;
                    return Err(BuilderError::DuplicatePath {
                        path: format_path(&d.full_path),
                    });
                }
            }
        }

        // Validate rule capture compatibility
        for decl in &self.declarations {
            for rule in &decl.rules {
                if rule.needs().is_capture() && !decl.is_capture {
                    return Err(BuilderError::IncompatibleAccessMode {
                        node_path: format_path(&decl.full_path),
                        rule_name: rule.name().to_owned(),
                    });
                }
            }
        }

        // Check for nested capture
        for decl in &self.declarations {
            if decl.is_capture {
                let mut ancestor_idx = decl.parent_index;
                while let Some(idx) = ancestor_idx {
                    if self.declarations[idx].is_capture {
                        return Err(BuilderError::NestedCapture {
                            inner: format_path(&decl.full_path),
                            outer: format_path(&self.declarations[idx].full_path),
                        });
                    }
                    ancestor_idx = self.declarations[idx].parent_index;
                }
            }
        }

        let mut nodes: Vec<DescriptorNode<R>> = Vec::with_capacity(self.declarations.len());
        for decl in self.declarations {
            let mut needs = decl.rules.iter().fold(NodeNeeds::empty(), |acc, r| acc | r.needs());
            // If the node is capture mode, ensure the CAPTURE bit is set in needs
            if decl.is_capture {
                needs |= NodeNeeds::CAPTURE;
            }
            nodes.push(DescriptorNode {
                full_path: decl.full_path,
                rules: decl.rules,
                children_sorted: Vec::new(),
                child_ids: Vec::new(),
                tag: decl.tag,
                parent_id: decl.parent_index.map(|i| NodeId(i as u32)),
                needs,
                summary_needs: NodeNeeds::empty(),
            });
        }

        // Build child relationships
        for i in 0..nodes.len() {
            if let Some(parent_id) = nodes[i].parent_id {
                let child_id = NodeId(i as u32);
                let child_tag = nodes[i].tag.clone();
                nodes[parent_id.0 as usize].child_ids.push(child_id);
                nodes[parent_id.0 as usize].children_sorted.push((child_tag, child_id));
            }
        }

        // Sort children_sorted by tag name for binary search
        for node in &mut nodes {
            node.children_sorted.sort_by(|(a, _), (b, _)| a.cmp(b));
        }

        // Propagate needs upward: if any node has declared children in the tree,
        // keep attrs and text so descendants can access them.
        for node in &mut nodes {
            if !node.child_ids.is_empty() {
                node.needs = node.needs | NodeNeeds::ATTRS | NodeNeeds::TEXT;
            }
        }

        // Compute summary_needs: what does the parent need from this child?
        for i in 0..nodes.len() {
            if let Some(parent_id) = nodes[i].parent_id {
                let parent_needs = nodes[parent_id.0 as usize].needs;
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
