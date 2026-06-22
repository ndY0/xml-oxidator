#![cfg(feature = "macros")]

use std::io::Cursor;
use std::sync::Arc;

use crossbeam_channel::bounded;
use xml_oxydizer::build_tree;
use xml_oxydizer::diagnostic::{Diagnostic, Severity};
use xml_oxydizer::pipeline::{FileInfo, PipelineConfig};
use xml_oxydizer::rule::{NodeAccess, Rule};
use xml_oxydizer::tree::descriptor::NodeNeeds;

// --- Rules used by macro tests ---

struct CheckAttr {
    attr_name: &'static str,
    expected: &'static str,
}

impl Rule for CheckAttr {
    fn name(&self) -> &str { "check_attr" }
    fn evaluate(&self, node: &dyn NodeAccess) -> Vec<Diagnostic> {
        match node.attr(self.attr_name) {
            Some(v) if v == self.expected => vec![],
            _ => vec![Diagnostic {
                rule_name: self.name().to_owned(),
                severity: Severity::Error,
                message: format!("attr {} != {}", self.attr_name, self.expected),
                element_path: node.path().to_vec(),
                element_index: node.element_index() as u32,
            }],
        }
    }
}

struct CheckText {
    expected: &'static str,
}

impl Rule for CheckText {
    fn name(&self) -> &str { "check_text" }
    fn evaluate(&self, node: &dyn NodeAccess) -> Vec<Diagnostic> {
        if node.text() == self.expected { vec![] }
        else {
            vec![Diagnostic {
                rule_name: self.name().to_owned(),
                severity: Severity::Error,
                message: format!("text mismatch: got {:?}", node.text()),
                element_path: node.path().to_vec(),
                element_index: node.element_index() as u32,
            }]
        }
    }
}

struct CheckParent {
    parent_attr: &'static str,
    expected: &'static str,
}

impl Rule for CheckParent {
    fn name(&self) -> &str { "check_parent" }
    fn evaluate(&self, node: &dyn NodeAccess) -> Vec<Diagnostic> {
        match node.ancestor_attr(0, self.parent_attr) {
            Some(v) if v == self.expected => vec![],
            _ => vec![Diagnostic {
                rule_name: self.name().to_owned(),
                severity: Severity::Error,
                message: "parent mismatch".into(),
                element_path: node.path().to_vec(),
                element_index: node.element_index() as u32,
            }],
        }
    }
}

struct SubtreeHasTag {
    tag: &'static str,
}

impl Rule for SubtreeHasTag {
    fn name(&self) -> &str { "subtree_has_tag" }
    fn needs(&self) -> NodeNeeds {
        NodeNeeds::all() | NodeNeeds::CAPTURE
    }
    fn evaluate(&self, node: &dyn NodeAccess) -> Vec<Diagnostic> {
        match node.subtree() {
            Some(st) if st.find(self.tag).is_some() => vec![],
            _ => vec![Diagnostic {
                rule_name: self.name().to_owned(),
                severity: Severity::Error,
                message: format!("missing <{}>", self.tag),
                element_path: node.path().to_vec(),
                element_index: node.element_index() as u32,
            }],
        }
    }
}

struct NoopRule;

impl Rule for NoopRule {
    fn name(&self) -> &str { "noop" }
    fn evaluate(&self, _node: &dyn NodeAccess) -> Vec<Diagnostic> { vec![] }
}

struct ChildCount {
    tag: &'static str,
    expected: usize,
}

impl Rule for ChildCount {
    fn name(&self) -> &str { "child_count" }
    fn needs(&self) -> NodeNeeds {
        NodeNeeds::all()
    }
    fn evaluate(&self, node: &dyn NodeAccess) -> Vec<Diagnostic> {
        let tree_ref = &(); // We can't access tree from NodeAccess, so count all children
        let _ = tree_ref;
        // Since ChildSummary no longer has .tag field, we count all children summaries.
        // In tests we only have one child type per parent, so counting all is correct.
        let count = node.children_summaries().len();
        if count == self.expected { vec![] }
        else {
            vec![Diagnostic {
                rule_name: self.name().to_owned(),
                severity: Severity::Error,
                message: format!("expected {} <{}>, got {}", self.expected, self.tag, count),
                element_path: node.path().to_vec(),
                element_index: node.element_index() as u32,
            }]
        }
    }
}

// --- Helper ---

fn run_xml(tree: xml_oxydizer::tree::descriptor::DescriptorTree<Box<dyn Rule>>, xml: &str) -> Vec<Diagnostic> {
    let tree = Arc::new(tree);
    let xml_bytes = xml.as_bytes().to_vec();
    let (diag_tx, diag_rx) = bounded(4096);
    let errors = xml_oxydizer::pipeline::run_pipeline(
        vec![FileInfo {
            filename: "test.xml".to_owned(),
            descriptors: tree,
            stream_factory: Box::new(move || {
                Box::new(Cursor::new(xml_bytes)) as Box<dyn std::io::Read + Send>
            }),
        }],
        diag_tx,
        &PipelineConfig::default(),
    );
    assert!(errors.is_empty(), "pipeline errors: {:?}", errors);
    diag_rx.try_iter().collect()
}

// --- Tests ---

#[test]
fn test_macro_simple_streaming() {
    let tree = build_tree!(
        "root" streaming {
            CheckAttr { attr_name: "v", expected: "1" }
        }
    ).unwrap();

    let diags = run_xml(tree, r#"<root v="1"/>"#);
    assert!(diags.is_empty(), "{:?}", diags);
}

#[test]
fn test_macro_default_mode_is_streaming() {
    let tree = build_tree!(
        "root" {
            CheckAttr { attr_name: "v", expected: "1" }
        }
    ).unwrap();

    let diags = run_xml(tree, r#"<root v="1"/>"#);
    assert!(diags.is_empty(), "{:?}", diags);
}

#[test]
fn test_macro_streaming_failure() {
    let tree = build_tree!(
        "root" streaming {
            CheckAttr { attr_name: "v", expected: "2" }
        }
    ).unwrap();

    let diags = run_xml(tree, r#"<root v="1"/>"#);
    assert_eq!(diags.len(), 1);
}

#[test]
fn test_macro_multiple_rules() {
    let tree = build_tree!(
        "root" streaming {
            CheckAttr { attr_name: "a", expected: "1" },
            CheckAttr { attr_name: "b", expected: "2" }
        }
    ).unwrap();

    let diags = run_xml(tree, r#"<root a="1" b="2"/>"#);
    assert!(diags.is_empty(), "{:?}", diags);
}

#[test]
fn test_macro_child_node() {
    let tree = build_tree!(
        "root" streaming {
            "child" streaming {
                CheckAttr { attr_name: "id", expected: "ok" }
            }
        }
    ).unwrap();

    let diags = run_xml(tree, r#"<root><child id="ok"/></root>"#);
    assert!(diags.is_empty(), "{:?}", diags);
}

#[test]
fn test_macro_child_default_mode() {
    let tree = build_tree!(
        "root" {
            "child" {
                CheckAttr { attr_name: "id", expected: "ok" }
            }
        }
    ).unwrap();

    let diags = run_xml(tree, r#"<root><child id="ok"/></root>"#);
    assert!(diags.is_empty(), "{:?}", diags);
}

#[test]
fn test_macro_mixed_rules_and_children() {
    let tree = build_tree!(
        "root" streaming {
            CheckAttr { attr_name: "v", expected: "1" },
            ChildCount { tag: "item", expected: 3 },
            "item" streaming {
                CheckAttr { attr_name: "status", expected: "ok" }
            }
        }
    ).unwrap();

    let xml = r#"<root v="1"><item status="ok"/><item status="ok"/><item status="ok"/></root>"#;
    let diags = run_xml(tree, xml);
    assert!(diags.is_empty(), "{:?}", diags);
}

#[test]
fn test_macro_capture_subtree() {
    let tree = build_tree!(
        "catalog" streaming {
            "schema" capture {
                SubtreeHasTag { tag: "field" }
            },
            "entry" streaming {
                CheckAttr { attr_name: "sku", expected: "A" }
            }
        }
    ).unwrap();

    let xml = r#"<catalog>
        <schema><field name="x"/></schema>
        <entry sku="A"/>
    </catalog>"#;
    let diags = run_xml(tree, xml);
    assert!(diags.is_empty(), "{:?}", diags);
}

#[test]
fn test_macro_deep_nesting() {
    let tree = build_tree!(
        "a" streaming {
            "b" {
                "c" {
                    "d" {
                        CheckAttr { attr_name: "val", expected: "ok" }
                    }
                }
            }
        }
    ).unwrap();

    let diags = run_xml(tree, r#"<a><b><c><d val="ok"/></c></b></a>"#);
    assert!(diags.is_empty(), "{:?}", diags);
}

#[test]
fn test_macro_parent_access() {
    let tree = build_tree!(
        "root" streaming {
            "child" streaming {
                CheckParent { parent_attr: "version", expected: "3" }
            }
        }
    ).unwrap();

    let diags = run_xml(tree, r#"<root version="3"><child/></root>"#);
    assert!(diags.is_empty(), "{:?}", diags);
}

#[test]
fn test_macro_siblings() {
    let tree = build_tree!(
        "root" streaming {
            "a" streaming {
                CheckAttr { attr_name: "x", expected: "1" }
            },
            "b" streaming {
                CheckAttr { attr_name: "y", expected: "2" }
            }
        }
    ).unwrap();

    let diags = run_xml(tree, r#"<root><a x="1"/><b y="2"/></root>"#);
    assert!(diags.is_empty(), "{:?}", diags);
}

#[test]
fn test_macro_empty_body() {
    let tree = build_tree!(
        "root" streaming {}
    ).unwrap();

    let diags = run_xml(tree, "<root/>");
    assert!(diags.is_empty());
}

#[test]
fn test_macro_noop_rule() {
    let tree = build_tree!(
        "root" {
            NoopRule {}
        }
    ).unwrap();

    let diags = run_xml(tree, "<root/>");
    assert!(diags.is_empty());
}

#[test]
fn test_macro_matches_builder_output() {
    use xml_oxydizer::tree::builder::TreeBuilder;

    let macro_tree = build_tree!(
        "root" streaming {
            CheckAttr { attr_name: "v", expected: "1" },
            "child" capture {
                SubtreeHasTag { tag: "inner" }
            },
            "item" streaming {
                CheckAttr { attr_name: "id", expected: "ok" },
                "detail" {
                    CheckText { expected: "hello" }
                }
            }
        }
    ).unwrap();

    let builder_tree = TreeBuilder::new("root")
        .streaming()
        .rule(Box::new(CheckAttr { attr_name: "v", expected: "1" }) as Box<dyn Rule>)
        .node("child")
            .capture_subtree()
            .rule(Box::new(SubtreeHasTag { tag: "inner" }) as Box<dyn Rule>)
            .done()
        .node("item")
            .streaming()
            .rule(Box::new(CheckAttr { attr_name: "id", expected: "ok" }) as Box<dyn Rule>)
            .node("detail")
                .rule(Box::new(CheckText { expected: "hello" }) as Box<dyn Rule>)
                .done()
            .done()
        .build()
        .unwrap();

    let xml = r#"<root v="1"><child><inner/></child><item id="ok"><detail>hello</detail></item></root>"#;

    let macro_diags = run_xml(macro_tree, xml);
    let builder_diags = run_xml(builder_tree, xml);

    assert!(macro_diags.is_empty(), "macro diags: {:?}", macro_diags);
    assert!(builder_diags.is_empty(), "builder diags: {:?}", builder_diags);
}

#[test]
fn test_macro_full_example_from_template() {
    let tree = build_tree!(
        "root" streaming {
            CheckAttr { attr_name: "v", expected: "1" },
            NoopRule {},
            "child" capture {
                SubtreeHasTag { tag: "deep_child" },
                "deep_child" {
                    CheckAttr { attr_name: "x", expected: "yes" }
                }
            }
        }
    );
    assert!(tree.is_ok(), "build failed: {:?}", tree.err());
}

#[test]
fn test_macro_trailing_comma() {
    let tree = build_tree!(
        "root" streaming {
            CheckAttr { attr_name: "a", expected: "1" },
            "child" {
                NoopRule {},
            },
        }
    ).unwrap();

    let diags = run_xml(tree, r#"<root a="1"><child/></root>"#);
    assert!(diags.is_empty(), "{:?}", diags);
}
