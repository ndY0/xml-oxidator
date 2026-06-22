use crate::diagnostic::Diagnostic;
use crate::tree::descriptor::NodeNeeds;
use crate::tree::path::PathSegment;
use crate::view::{ChildSummary, SubtreeNode};

pub trait NodeAccess {
    fn tag(&self) -> &str;
    fn attr(&self, name: &str) -> Option<&str>;
    fn for_each_attr(&self, f: &mut dyn FnMut(&str, &str));
    fn text(&self) -> &str;
    fn element_index(&self) -> usize;
    fn path(&self) -> &[PathSegment];
    fn children_summaries(&self) -> &[ChildSummary];
    fn subtree(&self) -> Option<&SubtreeNode<'_>>;
    fn depth(&self) -> usize;
    fn ancestor_attr(&self, level: usize, name: &str) -> Option<&str>;
    fn ancestor_text(&self, level: usize) -> Option<&str>;
    fn ancestor_children(&self, level: usize) -> Option<&[ChildSummary]>;
}

pub trait Rule: Send + Sync {
    fn name(&self) -> &str;
    fn evaluate(&self, node: &dyn NodeAccess) -> Vec<Diagnostic>;

    /// Declares what data this rule reads from the element.
    /// Defaults to all data (ATTRS | TEXT | CHILDREN).
    /// Set the CAPTURE flag if this rule requires subtree capture.
    fn needs(&self) -> NodeNeeds {
        NodeNeeds::all()
    }
}

// Blanket impl so that `Box<dyn Rule>` is itself a `Rule`.
// This allows `TreeBuilder<Box<dyn Rule>>` to work with heterogeneous rule sets.
impl Rule for Box<dyn Rule> {
    #[inline]
    fn name(&self) -> &str {
        (**self).name()
    }
    #[inline]
    fn evaluate(&self, node: &dyn NodeAccess) -> Vec<Diagnostic> {
        (**self).evaluate(node)
    }
    #[inline]
    fn needs(&self) -> NodeNeeds {
        (**self).needs()
    }
}
