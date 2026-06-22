use std::io::Cursor;
use std::sync::Arc;

use crossbeam_channel::bounded;
use xml_oxydizer::diagnostic::{Diagnostic, Severity};
use xml_oxydizer::error::BuilderError;
use xml_oxydizer::pipeline::{FileInfo, PipelineConfig};
use xml_oxydizer::rule::{NodeAccess, Rule};
use xml_oxydizer::tree::builder::TreeBuilder;
use xml_oxydizer::tree::descriptor::AccessMode;

#[cfg(feature = "test-heap")]
use dhat::Alloc;

#[cfg(feature = "test-heap")]
#[global_allocator]
static ALLOC: Alloc = Alloc;

// --- Test rules ---

struct CheckAttr {
    name: &'static str,
    attr_name: &'static str,
    expected_value: &'static str,
}

impl Rule for CheckAttr {
    fn name(&self) -> &str {
        self.name
    }
    fn access_mode(&self) -> AccessMode {
        AccessMode::Streaming
    }
    fn evaluate(&self, node: &dyn NodeAccess) -> Vec<Diagnostic> {
        match node.attr(self.attr_name) {
            Some(v) if v == self.expected_value => vec![],
            other => vec![Diagnostic {
                rule_name: self.name().to_owned(),
                severity: Severity::Error,
                message: format!(
                    "expected {}=\"{}\", got {:?}",
                    self.attr_name,
                    self.expected_value,
                    other
                ),
                element_path: node.path().to_vec(),
                element_index: node.element_index(),
            }],
        }
    }
}

struct CheckParentAttr {
    name: &'static str,
    parent_attr: &'static str,
    expected_value: &'static str,
}

impl Rule for CheckParentAttr {
    fn name(&self) -> &str {
        self.name
    }
    fn access_mode(&self) -> AccessMode {
        AccessMode::Streaming
    }
    fn evaluate(&self, node: &dyn NodeAccess) -> Vec<Diagnostic> {
        match node.ancestor_attr(0, self.parent_attr) {
            Some(v) if v == self.expected_value => vec![],
            other => vec![Diagnostic {
                rule_name: self.name().to_owned(),
                severity: Severity::Error,
                message: format!(
                    "parent attr {}=\"{}\", got {:?}",
                    self.parent_attr, self.expected_value, other
                ),
                element_path: node.path().to_vec(),
                element_index: node.element_index(),
            }],
        }
    }
}

struct CheckText {
    name: &'static str,
    expected: &'static str,
}

impl Rule for CheckText {
    fn name(&self) -> &str {
        self.name
    }
    fn access_mode(&self) -> AccessMode {
        AccessMode::Streaming
    }
    fn evaluate(&self, node: &dyn NodeAccess) -> Vec<Diagnostic> {
        if node.text() == self.expected {
            vec![]
        } else {
            vec![Diagnostic {
                rule_name: self.name().to_owned(),
                severity: Severity::Error,
                message: format!("expected text \"{}\", got \"{}\"", self.expected, node.text()),
                element_path: node.path().to_vec(),
                element_index: node.element_index(),
            }]
        }
    }
}

struct CheckChildCount {
    name: &'static str,
    child_tag: &'static str,
    expected_count: usize,
}

impl Rule for CheckChildCount {
    fn name(&self) -> &str {
        self.name
    }
    fn access_mode(&self) -> AccessMode {
        AccessMode::Streaming
    }
    fn evaluate(&self, node: &dyn NodeAccess) -> Vec<Diagnostic> {
        let count = node
            .children_summaries()
            .iter()
            .filter(|c| c.tag.0.as_ref() == self.child_tag)
            .count();
        if count == self.expected_count {
            vec![]
        } else {
            vec![Diagnostic {
                rule_name: self.name().to_owned(),
                severity: Severity::Error,
                message: format!(
                    "expected {} {} children, got {}",
                    self.expected_count, self.child_tag, count
                ),
                element_path: node.path().to_vec(),
                element_index: node.element_index(),
            }]
        }
    }
}

struct CheckSubtreeHasChild {
    name: &'static str,
    child_tag: &'static str,
}

impl Rule for CheckSubtreeHasChild {
    fn name(&self) -> &str {
        self.name
    }
    fn access_mode(&self) -> AccessMode {
        AccessMode::CaptureSubtree
    }
    fn evaluate(&self, node: &dyn NodeAccess) -> Vec<Diagnostic> {
        match node.subtree() {
            Some(subtree) => {
                if subtree.find(self.child_tag).is_some() {
                    vec![]
                } else {
                    vec![Diagnostic {
                        rule_name: self.name().to_owned(),
                        severity: Severity::Error,
                        message: format!("subtree missing child <{}>", self.child_tag),
                        element_path: node.path().to_vec(),
                        element_index: node.element_index(),
                    }]
                }
            }
            None => vec![Diagnostic {
                rule_name: self.name().to_owned(),
                severity: Severity::Error,
                message: "no subtree available".to_owned(),
                element_path: node.path().to_vec(),
                element_index: node.element_index(),
            }],
        }
    }
}

struct CountSubtreeDescendants {
    name: &'static str,
    tag: &'static str,
    expected: usize,
}

impl Rule for CountSubtreeDescendants {
    fn name(&self) -> &str {
        self.name
    }
    fn access_mode(&self) -> AccessMode {
        AccessMode::CaptureSubtree
    }
    fn evaluate(&self, node: &dyn NodeAccess) -> Vec<Diagnostic> {
        let count = node
            .subtree()
            .map(|s| s.find_all(self.tag).len())
            .unwrap_or(0);
        if count == self.expected {
            vec![]
        } else {
            vec![Diagnostic {
                rule_name: self.name().to_owned(),
                severity: Severity::Error,
                message: format!(
                    "expected {} <{}> descendants, got {}",
                    self.expected, self.tag, count
                ),
                element_path: node.path().to_vec(),
                element_index: node.element_index(),
            }]
        }
    }
}

// --- Tests ---

#[test]
fn test_streaming_10k_children() {
    #[cfg(feature = "test-heap")]
    let _profiler = dhat::Profiler::new_heap();

    let tree = TreeBuilder::new("root")
        .streaming()
        .rule(Box::new(CheckAttr {
            name: "check_root_attr",
            attr_name: "test",
            expected_value: "value",
        }))
        .rule(Box::new(CheckChildCount {
            name: "check_child_count",
            child_tag: "child",
            expected_count: 10_000,
        }))
        .node("child")
            .streaming()
            .rule(Box::new(CheckAttr {
                name: "check_child_attr",
                attr_name: "test2",
                expected_value: "value2",
            }))
            .done()
        .build()
        .unwrap();

    let tree = Arc::new(tree);

    let mut xml = String::from(r#"<root test="value">"#);
    for _ in 0..10_000 {
        xml.push_str(r#"<child test2="value2"></child>"#);
    }
    xml.push_str("</root>");

    let xml_bytes = xml.into_bytes();

    let (diag_tx, diag_rx) = bounded(1024);

    let errors = xml_oxydizer::pipeline::run_pipeline(
        vec![FileInfo {
            filename: "test.xml".to_owned(),
            descriptors: Arc::clone(&tree),
            stream_factory: Box::new(move || Box::new(Cursor::new(xml_bytes)) as Box<dyn std::io::Read + Send>),
        }],
        diag_tx,
        &PipelineConfig::default(),
    );

    assert!(errors.is_empty(), "pipeline errors: {:?}", errors);

    let diagnostics: Vec<Diagnostic> = diag_rx.try_iter().collect();
    assert!(diagnostics.is_empty(), "unexpected diagnostics: {:?}", diagnostics);
}

#[test]
fn test_streaming_with_failures() {
    let tree = TreeBuilder::new("root")
        .streaming()
        .rule(Box::new(CheckAttr {
            name: "check_root",
            attr_name: "version",
            expected_value: "2",
        }))
        .node("item")
            .streaming()
            .rule(Box::new(CheckAttr {
                name: "check_item",
                attr_name: "status",
                expected_value: "active",
            }))
            .done()
        .build()
        .unwrap();

    let xml = r#"<root version="1"><item status="active"></item><item status="inactive"></item></root>"#;

    let (diag_tx, diag_rx) = bounded(1024);
    let errors = xml_oxydizer::pipeline::run_pipeline(
        vec![FileInfo {
            filename: "test.xml".to_owned(),
            descriptors: Arc::new(tree),
            stream_factory: Box::new(move || {
                Box::new(Cursor::new(xml.as_bytes().to_vec())) as Box<dyn std::io::Read + Send>
            }),
        }],
        diag_tx,
        &PipelineConfig::default(),
    );

    assert!(errors.is_empty());

    let diagnostics: Vec<Diagnostic> = diag_rx.try_iter().collect();
    assert_eq!(diagnostics.len(), 2);
    assert!(diagnostics.iter().any(|d| d.rule_name == "check_root"));
    assert!(diagnostics.iter().any(|d| d.rule_name == "check_item" && d.element_index == 1));
}

#[test]
fn test_parent_access() {
    let tree = TreeBuilder::new("root")
        .streaming()
        .node("child")
            .streaming()
            .rule(Box::new(CheckParentAttr {
                name: "check_parent",
                parent_attr: "version",
                expected_value: "3",
            }))
            .done()
        .build()
        .unwrap();

    let xml = r#"<root version="3"><child></child></root>"#;

    let (diag_tx, diag_rx) = bounded(1024);
    let errors = xml_oxydizer::pipeline::run_pipeline(
        vec![FileInfo {
            filename: "test.xml".to_owned(),
            descriptors: Arc::new(tree),
            stream_factory: Box::new(move || {
                Box::new(Cursor::new(xml.as_bytes().to_vec())) as Box<dyn std::io::Read + Send>
            }),
        }],
        diag_tx,
        &PipelineConfig::default(),
    );

    assert!(errors.is_empty());
    let diagnostics: Vec<Diagnostic> = diag_rx.try_iter().collect();
    assert!(diagnostics.is_empty(), "unexpected diagnostics: {:?}", diagnostics);
}

#[test]
fn test_capture_subtree() {
    let tree = TreeBuilder::new("catalog")
        .streaming()
        .node("schema")
            .capture_subtree()
            .rule(Box::new(CheckSubtreeHasChild {
                name: "schema_has_field",
                child_tag: "field",
            }))
            .rule(Box::new(CountSubtreeDescendants {
                name: "schema_field_count",
                tag: "field",
                expected: 3,
            }))
            .done()
        .node("entry")
            .streaming()
            .rule(Box::new(CheckAttr {
                name: "check_entry_sku",
                attr_name: "sku",
                expected_value: "ABC",
            }))
            .done()
        .build()
        .unwrap();

    let xml = r#"<catalog>
        <schema>
            <field name="sku" type="string"/>
            <field name="price" type="decimal"/>
            <field name="name" type="string"/>
        </schema>
        <entry sku="ABC"></entry>
        <entry sku="ABC"></entry>
    </catalog>"#;

    let (diag_tx, diag_rx) = bounded(1024);
    let errors = xml_oxydizer::pipeline::run_pipeline(
        vec![FileInfo {
            filename: "catalog.xml".to_owned(),
            descriptors: Arc::new(tree),
            stream_factory: Box::new(move || {
                Box::new(Cursor::new(xml.as_bytes().to_vec())) as Box<dyn std::io::Read + Send>
            }),
        }],
        diag_tx,
        &PipelineConfig::default(),
    );

    assert!(errors.is_empty(), "pipeline errors: {:?}", errors);
    let diagnostics: Vec<Diagnostic> = diag_rx.try_iter().collect();
    assert!(diagnostics.is_empty(), "unexpected diagnostics: {:?}", diagnostics);
}

#[test]
fn test_capture_with_parent_access() {
    struct CaptureWithParent;

    impl Rule for CaptureWithParent {
        fn name(&self) -> &str {
            "capture_parent_check"
        }
        fn access_mode(&self) -> AccessMode {
            AccessMode::CaptureSubtree
        }
        fn evaluate(&self, node: &dyn NodeAccess) -> Vec<Diagnostic> {
            match node.ancestor_attr(0, "version") {
                Some("3") => vec![],
                _ => vec![Diagnostic {
                    rule_name: self.name().to_owned(),
                    severity: Severity::Error,
                    message: "parent missing version=3".to_owned(),
                    element_path: node.path().to_vec(),
                    element_index: node.element_index(),
                }],
            }
        }
    }

    let tree = TreeBuilder::new("root")
        .streaming()
        .node("section")
            .capture_subtree()
            .rule(Box::new(CaptureWithParent))
            .done()
        .build()
        .unwrap();

    let xml = r#"<root version="3"><section><item>hello</item></section></root>"#;

    let (diag_tx, diag_rx) = bounded(1024);
    let errors = xml_oxydizer::pipeline::run_pipeline(
        vec![FileInfo {
            filename: "test.xml".to_owned(),
            descriptors: Arc::new(tree),
            stream_factory: Box::new(move || {
                Box::new(Cursor::new(xml.as_bytes().to_vec())) as Box<dyn std::io::Read + Send>
            }),
        }],
        diag_tx,
        &PipelineConfig::default(),
    );

    assert!(errors.is_empty());
    let diagnostics: Vec<Diagnostic> = diag_rx.try_iter().collect();
    assert!(diagnostics.is_empty(), "unexpected diagnostics: {:?}", diagnostics);
}

#[test]
fn test_text_content() {
    let tree = TreeBuilder::new("root")
        .streaming()
        .node("message")
            .streaming()
            .rule(Box::new(CheckText {
                name: "check_msg",
                expected: "hello world",
            }))
            .done()
        .build()
        .unwrap();

    let xml = r#"<root><message>hello world</message></root>"#;

    let (diag_tx, diag_rx) = bounded(1024);
    let errors = xml_oxydizer::pipeline::run_pipeline(
        vec![FileInfo {
            filename: "test.xml".to_owned(),
            descriptors: Arc::new(tree),
            stream_factory: Box::new(move || {
                Box::new(Cursor::new(xml.as_bytes().to_vec())) as Box<dyn std::io::Read + Send>
            }),
        }],
        diag_tx,
        &PipelineConfig::default(),
    );

    assert!(errors.is_empty());
    let diagnostics: Vec<Diagnostic> = diag_rx.try_iter().collect();
    assert!(diagnostics.is_empty(), "unexpected diagnostics: {:?}", diagnostics);
}

#[test]
fn test_unmatched_elements_skipped() {
    let tree = TreeBuilder::new("root")
        .streaming()
        .node("target")
            .streaming()
            .rule(Box::new(CheckAttr {
                name: "check_target",
                attr_name: "ok",
                expected_value: "yes",
            }))
            .done()
        .build()
        .unwrap();

    let xml = r#"<root><ignored><nested>deep</nested></ignored><target ok="yes"></target><also_ignored/></root>"#;

    let (diag_tx, diag_rx) = bounded(1024);
    let errors = xml_oxydizer::pipeline::run_pipeline(
        vec![FileInfo {
            filename: "test.xml".to_owned(),
            descriptors: Arc::new(tree),
            stream_factory: Box::new(move || {
                Box::new(Cursor::new(xml.as_bytes().to_vec())) as Box<dyn std::io::Read + Send>
            }),
        }],
        diag_tx,
        &PipelineConfig::default(),
    );

    assert!(errors.is_empty());
    let diagnostics: Vec<Diagnostic> = diag_rx.try_iter().collect();
    assert!(diagnostics.is_empty(), "unexpected diagnostics: {:?}", diagnostics);
}

#[test]
fn test_self_closing_tags() {
    let tree = TreeBuilder::new("root")
        .streaming()
        .node("item")
            .streaming()
            .rule(Box::new(CheckAttr {
                name: "check_item",
                attr_name: "id",
                expected_value: "1",
            }))
            .done()
        .build()
        .unwrap();

    let xml = r#"<root><item id="1"/></root>"#;

    let (diag_tx, diag_rx) = bounded(1024);
    let errors = xml_oxydizer::pipeline::run_pipeline(
        vec![FileInfo {
            filename: "test.xml".to_owned(),
            descriptors: Arc::new(tree),
            stream_factory: Box::new(move || {
                Box::new(Cursor::new(xml.as_bytes().to_vec())) as Box<dyn std::io::Read + Send>
            }),
        }],
        diag_tx,
        &PipelineConfig::default(),
    );

    assert!(errors.is_empty());
    let diagnostics: Vec<Diagnostic> = diag_rx.try_iter().collect();
    assert!(diagnostics.is_empty(), "unexpected diagnostics: {:?}", diagnostics);
}

#[test]
fn test_capture_overflow() {
    let tree = TreeBuilder::new("root")
        .streaming()
        .node("big")
            .capture_subtree()
            .rule(Box::new(CheckSubtreeHasChild {
                name: "dummy",
                child_tag: "x",
            }))
            .done()
        .capture_limit(100)
        .build()
        .unwrap();

    let mut xml = String::from("<root><big>");
    for i in 0..100 {
        xml.push_str(&format!(r#"<item id="{}">some text content</item>"#, i));
    }
    xml.push_str("</big></root>");

    let xml_bytes = xml.into_bytes();

    let (diag_tx, _diag_rx) = bounded(1024);
    let errors = xml_oxydizer::pipeline::run_pipeline(
        vec![FileInfo {
            filename: "test.xml".to_owned(),
            descriptors: Arc::new(tree),
            stream_factory: Box::new(move || {
                Box::new(Cursor::new(xml_bytes)) as Box<dyn std::io::Read + Send>
            }),
        }],
        diag_tx,
        &PipelineConfig::default(),
    );

    assert_eq!(errors.len(), 1);
    let err_msg = format!("{}", errors[0]);
    assert!(err_msg.contains("capture buffer exceeded"), "got: {}", err_msg);
}

#[test]
fn test_multiple_files_parallel() {
    let tree = Arc::new(
        TreeBuilder::new("doc")
            .streaming()
            .rule(Box::new(CheckAttr {
                name: "check_version",
                attr_name: "v",
                expected_value: "1",
            }))
            .build()
            .unwrap(),
    );

    let (diag_tx, diag_rx) = bounded(4096);

    let files: Vec<FileInfo> = (0..100)
        .map(|i| {
            let tree = Arc::clone(&tree);
            let xml = format!(r#"<doc v="1">file {}</doc>"#, i);
            FileInfo {
                filename: format!("file_{}.xml", i),
                descriptors: tree,
                stream_factory: Box::new(move || {
                    Box::new(Cursor::new(xml.into_bytes())) as Box<dyn std::io::Read + Send>
                }),
            }
        })
        .collect();

    let errors = xml_oxydizer::pipeline::run_pipeline(files, diag_tx, &PipelineConfig::default());

    assert!(errors.is_empty(), "errors: {:?}", errors);
    let diagnostics: Vec<Diagnostic> = diag_rx.try_iter().collect();
    assert!(diagnostics.is_empty(), "unexpected diagnostics: {:?}", diagnostics);
}

#[test]
fn test_streaming_pipeline_with_channel() {
    let tree = Arc::new(
        TreeBuilder::new("item")
            .streaming()
            .rule(Box::new(CheckAttr {
                name: "check_id",
                attr_name: "id",
                expected_value: "ok",
            }))
            .build()
            .unwrap(),
    );

    let (file_tx, file_rx) = bounded(10);
    let (diag_tx, diag_rx) = bounded(1024);

    let sender = std::thread::spawn(move || {
        for i in 0..20 {
            let tree = Arc::clone(&tree);
            let xml = format!(r#"<item id="ok">item {}</item>"#, i);
            file_tx
                .send(FileInfo {
                    filename: format!("stream_{}.xml", i),
                    descriptors: tree,
                    stream_factory: Box::new(move || {
                        Box::new(Cursor::new(xml.into_bytes())) as Box<dyn std::io::Read + Send>
                    }),
                })
                .unwrap();
        }
    });

    let errors = xml_oxydizer::pipeline::run_pipeline_streaming(
        file_rx,
        diag_tx,
        &PipelineConfig::default(),
    );

    sender.join().unwrap();
    assert!(errors.is_empty());
    let diagnostics: Vec<Diagnostic> = diag_rx.try_iter().collect();
    assert!(diagnostics.is_empty(), "unexpected diagnostics: {:?}", diagnostics);
}

#[test]
fn test_lazy_loading() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let tree = Arc::new(
        TreeBuilder::new("root")
            .streaming()
            .build()
            .unwrap(),
    );

    let counter = Arc::new(AtomicUsize::new(0));
    let (diag_tx, _diag_rx) = bounded(1024);

    let files: Vec<FileInfo> = (0..5)
        .map(|i| {
            let tree = Arc::clone(&tree);
            let counter = Arc::clone(&counter);
            FileInfo {
                filename: format!("lazy_{}.xml", i),
                descriptors: tree,
                stream_factory: Box::new(move || {
                    counter.fetch_add(1, Ordering::SeqCst);
                    Box::new(Cursor::new(b"<root/>".to_vec())) as Box<dyn std::io::Read + Send>
                }),
            }
        })
        .collect();

    assert_eq!(counter.load(Ordering::SeqCst), 0, "factories should not be called yet");

    let errors = xml_oxydizer::pipeline::run_pipeline(files, diag_tx, &PipelineConfig::default());
    assert!(errors.is_empty());
    assert_eq!(counter.load(Ordering::SeqCst), 5, "all factories should be called after processing");
}

#[test]
fn test_builder_incompatible_access_mode() {
    let result = TreeBuilder::new("root")
        .streaming()
        .rule(Box::new(CheckSubtreeHasChild {
            name: "needs_capture",
            child_tag: "x",
        }))
        .build();

    assert!(matches!(result, Err(BuilderError::IncompatibleAccessMode { .. })));
}

#[test]
fn test_builder_nested_capture_rejected() {
    let result = TreeBuilder::new("root")
        .capture_subtree()
        .node("inner")
            .capture_subtree()
            .done()
        .build();

    assert!(matches!(result, Err(BuilderError::NestedCapture { .. })));
}

#[test]
fn test_sibling_visibility() {
    struct CheckSiblingCount {
        expected: usize,
    }

    impl Rule for CheckSiblingCount {
        fn name(&self) -> &str {
            "sibling_count"
        }
        fn access_mode(&self) -> AccessMode {
            AccessMode::Streaming
        }
        fn evaluate(&self, node: &dyn NodeAccess) -> Vec<Diagnostic> {
            let siblings = node
                .ancestor_children(0)
                .map(|c| c.len())
                .unwrap_or(0);
            if node.element_index() == 2 && siblings != self.expected {
                vec![Diagnostic {
                    rule_name: self.name().to_owned(),
                    severity: Severity::Error,
                    message: format!("expected {} previous siblings, got {}", self.expected, siblings),
                    element_path: node.path().to_vec(),
                    element_index: node.element_index(),
                }]
            } else {
                vec![]
            }
        }
    }

    let tree = TreeBuilder::new("root")
        .streaming()
        .node("item")
            .streaming()
            .rule(Box::new(CheckSiblingCount { expected: 2 }))
            .done()
        .build()
        .unwrap();

    let xml = r#"<root><item/><item/><item/></root>"#;

    let (diag_tx, diag_rx) = bounded(1024);
    let errors = xml_oxydizer::pipeline::run_pipeline(
        vec![FileInfo {
            filename: "test.xml".to_owned(),
            descriptors: Arc::new(tree),
            stream_factory: Box::new(move || {
                Box::new(Cursor::new(xml.as_bytes().to_vec())) as Box<dyn std::io::Read + Send>
            }),
        }],
        diag_tx,
        &PipelineConfig::default(),
    );

    assert!(errors.is_empty());
    let diagnostics: Vec<Diagnostic> = diag_rx.try_iter().collect();
    assert!(diagnostics.is_empty(), "unexpected diagnostics: {:?}", diagnostics);
}

#[test]
fn test_deep_nesting() {
    let tree = TreeBuilder::new("a")
        .streaming()
        .node("b")
            .streaming()
            .node("c")
                .streaming()
                .node("d")
                    .streaming()
                    .rule(Box::new(CheckAttr {
                        name: "check_d",
                        attr_name: "val",
                        expected_value: "ok",
                    }))
                    .done()
                .done()
            .done()
        .build()
        .unwrap();

    let xml = r#"<a><b><c><d val="ok"/></c></b></a>"#;

    let (diag_tx, diag_rx) = bounded(1024);
    let errors = xml_oxydizer::pipeline::run_pipeline(
        vec![FileInfo {
            filename: "test.xml".to_owned(),
            descriptors: Arc::new(tree),
            stream_factory: Box::new(move || {
                Box::new(Cursor::new(xml.as_bytes().to_vec())) as Box<dyn std::io::Read + Send>
            }),
        }],
        diag_tx,
        &PipelineConfig::default(),
    );

    assert!(errors.is_empty());
    let diagnostics: Vec<Diagnostic> = diag_rx.try_iter().collect();
    assert!(diagnostics.is_empty(), "unexpected diagnostics: {:?}", diagnostics);
}
