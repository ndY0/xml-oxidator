use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

#[derive(Clone, PartialEq, Eq)]
pub struct PathSegment(pub Arc<str>);

impl Hash for PathSegment {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.as_ref().hash(state);
    }
}

impl std::borrow::Borrow<str> for PathSegment {
    #[inline]
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
    #[inline]
    fn from(s: &str) -> Self {
        Self(Arc::from(s))
    }
}

impl From<String> for PathSegment {
    #[inline]
    fn from(s: String) -> Self {
        Self(Arc::from(s.as_str()))
    }
}

impl PartialOrd for PathSegment {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PathSegment {
    #[inline]
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.as_ref().cmp(other.0.as_ref())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct NodeId(pub(crate) u32);

#[inline]
pub fn format_path(segments: &[PathSegment]) -> String {
    segments
        .iter()
        .map(|s| s.0.as_ref())
        .collect::<Vec<_>>()
        .join("/")
}
