use crate::tree::path::NodeId;
use crate::view::ChildSummary;

pub struct NodeContext {
    // Heap-allocated fields first
    pub attrs: Vec<(String, String)>,
    pub text: String,
    pub children: Vec<ChildSummary>,
    // Then smaller fields
    pub descriptor_id: NodeId,
    pub index: u32,
}

pub(crate) struct ContextPool {
    free: Vec<NodeContext>,
}

impl ContextPool {
    #[inline]
    pub fn new() -> Self {
        Self { free: Vec::new() }
    }

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

    #[inline]
    pub fn release(&mut self, mut ctx: NodeContext) {
        ctx.attrs.clear();
        ctx.text.clear();
        ctx.children.clear();
        self.free.push(ctx);
    }
}

pub(crate) struct AttrPool {
    free: Vec<Vec<(String, String)>>,
}

impl AttrPool {
    #[inline]
    pub fn new() -> Self {
        Self { free: Vec::new() }
    }

    #[inline]
    pub fn acquire(&mut self) -> Vec<(String, String)> {
        self.free.pop().unwrap_or_default()
    }
}
