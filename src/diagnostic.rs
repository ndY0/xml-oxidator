use crate::tree::path::PathSegment;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone)]
pub struct Diagnostic {
    // Vecs/Strings first (pointer-sized, 8-byte aligned)
    pub rule_name: String,
    pub message: String,
    pub element_path: Vec<PathSegment>,
    // Then u32
    pub element_index: u32,
    // Then smaller types
    pub severity: Severity,
}
