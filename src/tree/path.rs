use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

#[derive(Clone, PartialEq, Eq)]
pub struct PathSegment(pub Arc<str>);

impl Hash for PathSegment {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.as_ref().hash(state);
    }
}

impl std::borrow::Borrow<str> for PathSegment {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for PathSegment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PathSegment(\"{}\")", self.0)
    }
}

impl fmt::Display for PathSegment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for PathSegment {
    fn from(s: &str) -> Self {
        Self(Arc::from(s))
    }
}

impl From<String> for PathSegment {
    fn from(s: String) -> Self {
        Self(Arc::from(s.as_str()))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct NodeId(pub(crate) usize);

pub fn format_path(segments: &[PathSegment]) -> String {
    segments
        .iter()
        .map(|s| s.0.as_ref())
        .collect::<Vec<_>>()
        .join("/")
}
