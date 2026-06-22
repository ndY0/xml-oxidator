use crate::diagnostic::Diagnostic;
use crate::tree::descriptor::NodeNeeds;
use crate::tree::path::PathSegment;
use crate::view::{ChildSummary, SubtreeNode};

/// Unified read-only interface for accessing an XML element during rule evaluation.
///
/// Implemented by both [`StreamingView`](crate::view::StreamingView) and
/// [`SubtreeView`](crate::view::SubtreeView), allowing rules to be written
/// independently of the access mode.
pub trait NodeAccess {
    /// XML tag name of this element.
    fn tag(&self) -> &str;
    /// Looks up a single attribute by name.
    fn attr(&self, name: &str) -> Option<&str>;
    /// Iterates over all attributes as (key, value) pairs.
    fn for_each_attr(&self, f: &mut dyn FnMut(&str, &str));
    /// Direct text content of this element (not recursive).
    fn text(&self) -> &str;
    /// Zero-based sibling index among elements with the same tag under the same parent.
    fn element_index(&self) -> usize;
    /// Full path from root to this element as path segments.
    fn path(&self) -> &[PathSegment];
    /// Summaries of completed child elements in document order.
    fn children_summaries(&self) -> &[ChildSummary];
    /// Returns the captured subtree root, or `None` if this is a streaming-mode view.
    fn subtree(&self) -> Option<&SubtreeNode<'_>>;
    /// Depth of this element in the XML tree (0 = root).
    fn depth(&self) -> usize;
    /// Looks up an attribute on an ancestor. `level` 0 = parent, 1 = grandparent, etc.
    fn ancestor_attr(&self, level: usize, name: &str) -> Option<&str>;
    /// Returns the text content of an ancestor. `level` 0 = parent, 1 = grandparent, etc.
    fn ancestor_text(&self, level: usize) -> Option<&str>;
    /// Returns children summaries of an ancestor. `level` 0 = parent, 1 = grandparent, etc.
    fn ancestor_children(&self, level: usize) -> Option<&[ChildSummary]>;
}

/// A synchronous, stateless validation rule evaluated at element close time.
///
/// Rules must be `Send + Sync` to allow file-level parallelism via rayon.
pub trait Rule: Send + Sync {
    /// Unique name identifying this rule (used in diagnostic output).
    fn name(&self) -> &str;
    /// Evaluates this rule against the given element, returning any diagnostics.
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
