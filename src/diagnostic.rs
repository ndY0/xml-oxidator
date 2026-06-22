use crate::tree::path::PathSegment;

/// Severity level of a validation diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// A hard failure that must be addressed.
    Error,
    /// A potential issue that should be reviewed.
    Warning,
    /// An informational note with no required action.
    Info,
}

/// A single validation finding emitted by a rule during XML parsing.
#[derive(Debug, Clone)]
pub struct Diagnostic {
    /// Name of the rule that produced this diagnostic.
    pub rule_name: String,
    /// Human-readable description of the finding.
    pub message: String,
    /// Full path from root to the element that triggered the diagnostic.
    pub element_path: Vec<PathSegment>,
    /// Zero-based sibling index of the element among same-tag siblings.
    pub element_index: u32,
    /// Severity level of this diagnostic.
    pub severity: Severity,
}
