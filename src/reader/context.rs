use crate::tree::path::NodeId;
use crate::view::ChildSummary;

/// Per-element stack frame holding runtime state during streaming parsing.
///
/// One `NodeContext` is pushed when a tracked element opens and popped at close.
/// Accumulates attrs, text, and child summaries for rule evaluation.
pub struct NodeContext {
    /// Attributes of this element.
    pub attrs: Vec<(String, String)>,
    /// Accumulated direct text content.
    pub text: String,
    /// Completed child summaries in document order.
    pub children: Vec<ChildSummary>,
    /// Descriptor node ID linking this context to its declaration.
    pub descriptor_id: NodeId,
    /// Zero-based sibling index among same-tag siblings.
    pub index: u32,
}

/// Object pool for [`NodeContext`] instances to reduce heap allocations.
pub(crate) struct ContextPool {
    free: Vec<NodeContext>,
}

impl ContextPool {
    /// Creates an empty pool.
    #[inline]
    pub fn new() -> Self {
        Self { free: Vec::new() }
    }

    /// Returns a recycled or new `NodeContext` initialized with the given values.
    #[inline]
    pub fn acquire(
        &mut self,
        descriptor_id: NodeId,
        attrs: Vec<(String, String)>,
        index: u32,
    ) -> NodeContext {
        if let Some(mut ctx) = self.free.pop() {
            ctx.descriptor_id = descriptor_id;
            ctx.attrs = attrs;
            ctx.text.clear();
            ctx.children.clear();
            ctx.index = index;
            ctx
        } else {
            NodeContext {
                descriptor_id,
                attrs,
                text: String::new(),
                children: Vec::new(),
                index,
            }
        }
    }

    /// Returns a `NodeContext` to the pool for reuse.
    #[inline]
    pub fn release(&mut self, mut ctx: NodeContext) {
        ctx.attrs.clear();
        ctx.text.clear();
        ctx.children.clear();
        self.free.push(ctx);
    }
}

/// Object pool for attribute vectors to reduce heap allocations.
pub(crate) struct AttrPool {
    free: Vec<Vec<(String, String)>>,
}

impl AttrPool {
    /// Creates an empty pool.
    #[inline]
    pub fn new() -> Self {
        Self { free: Vec::new() }
    }

    /// Returns a recycled or new attribute vector.
    #[inline]
    pub fn acquire(&mut self) -> Vec<(String, String)> {
        self.free.pop().unwrap_or_default()
    }
}
