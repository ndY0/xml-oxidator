use crate::tree::path::PathSegment;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub rule_name: String,
    pub severity: Severity,
    pub message: String,
    pub element_path: Vec<PathSegment>,
    pub element_index: usize,
}
