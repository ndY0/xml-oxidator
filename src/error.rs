use thiserror::Error;

/// Errors that occur during [`DescriptorTree`](crate::tree::descriptor::DescriptorTree) construction.
#[derive(Debug, Error)]
pub enum BuilderError {
    #[error("no root node declared")]
    NoRoot,

    #[error("orphan node at path '{path}': parent '{parent_path}' not declared")]
    OrphanNode { path: String, parent_path: String },

    #[error("rule '{rule_name}' on node '{node_path}' requires CaptureSubtree, but node is Streaming")]
    IncompatibleAccessMode { node_path: String, rule_name: String },

    #[error("duplicate node declaration for path '{path}'")]
    DuplicatePath { path: String },

    #[error("nested capture: '{inner}' is CaptureSubtree within already-captured ancestor '{outer}'")]
    NestedCapture { inner: String, outer: String },
}

/// Errors that occur during XML parsing and rule evaluation.
#[derive(Debug, Error)]
pub enum ReaderError {
    #[error("XML parse error: {source}")]
    XmlParse {
        #[from]
        source: quick_xml::Error,
    },

    #[error("I/O error: {source}")]
    Io {
        #[from]
        source: std::io::Error,
    },

    #[error("capture buffer exceeded limit of {limit} bytes for subtree at '{path}'")]
    CaptureOverflow { path: String, limit: usize },

    #[error("unexpected root element '{found}', expected '{expected}'")]
    RootMismatch { expected: String, found: String },

    #[error("UTF-8 decoding error: {source}")]
    Utf8 {
        #[from]
        source: std::str::Utf8Error,
    },
}

/// Top-level errors from the file-parallel validation pipeline.
#[derive(Debug, Error)]
pub enum PipelineError {
    #[error("reader error on file '{filename}': {source}")]
    Reader {
        filename: String,
        #[source]
        source: ReaderError,
    },
}
