use std::io::Cursor;
use std::sync::Arc;
use std::time::Duration;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use crossbeam_channel::bounded;
use xml_oxydizer::diagnostic::{Diagnostic, Severity};
use xml_oxydizer::pipeline::{FileInfo, PipelineConfig, run_pipeline, run_pipeline_streaming};
use xml_oxydizer::rule::{NodeAccess, Rule};
use xml_oxydizer::tree::builder::TreeBuilder;
use xml_oxydizer::tree::descriptor::AccessMode;

// ---------------------------------------------------------------------------
// Reusable rules
// ---------------------------------------------------------------------------

struct NoopRule;
impl Rule for NoopRule {
    fn name(&self) -> &str { "noop" }
    fn access_mode(&self) -> AccessMode { AccessMode::Streaming }
    fn evaluate(&self, _node: &dyn NodeAccess) -> Vec<Diagnostic> { vec![] }
}

struct CheckAttrRule { attr: &'static str, expected: &'static str }
impl Rule for CheckAttrRule {
    fn name(&self) -> &str { "check_attr" }
    fn access_mode(&self) -> AccessMode { AccessMode::Streaming }
    fn evaluate(&self, node: &dyn NodeAccess) -> Vec<Diagnostic> {
        match node.attr(self.attr) {
            Some(v) if v == self.expected => vec![],
            _ => vec![Diagnostic {
                rule_name: "check_attr".into(),
                severity: Severity::Error,
                message: "mismatch".into(),
                element_path: node.path().to_vec(),
                element_index: node.element_index(),
            }],
        }
    }
}

struct HeavyAttrRule;
impl Rule for HeavyAttrRule {
    fn name(&self) -> &str { "heavy_attr" }
    fn access_mode(&self) -> AccessMode { AccessMode::Streaming }
    fn evaluate(&self, node: &dyn NodeAccess) -> Vec<Diagnostic> {
        let mut hash: u64 = 0;
        node.for_each_attr(&mut |k, v| {
            for b in k.bytes().chain(v.bytes()) {
                hash = hash.wrapping_mul(31).wrapping_add(b as u64);
            }
        });
        for b in node.text().bytes() {
            hash = hash.wrapping_mul(31).wrapping_add(b as u64);
        }
        if let Some(parent_text) = node.ancestor_text(0) {
            // Access parent to simulate parent-dependent rule
            for b in parent_text.bytes() {
                hash = hash.wrapping_mul(31).wrapping_add(b as u64);
            }
        }
        std::hint::black_box(hash);
        vec![]
    }
}

struct ChildAggregateRule;
impl Rule for ChildAggregateRule {
    fn name(&self) -> &str { "child_aggregate" }
    fn access_mode(&self) -> AccessMode { AccessMode::Streaming }
    fn evaluate(&self, node: &dyn NodeAccess) -> Vec<Diagnostic> {
        let mut seen = std::collections::HashSet::new();
        for child in node.children_summaries() {
            if let Some(id) = child.attr("id") {
                seen.insert(id.to_owned());
            }
        }
        std::hint::black_box(seen.len());
        vec![]
    }
}

struct SubtreeCountRule { tag: &'static str }
impl Rule for SubtreeCountRule {
    fn name(&self) -> &str { "subtree_count" }
    fn access_mode(&self) -> AccessMode { AccessMode::CaptureSubtree }
    fn evaluate(&self, node: &dyn NodeAccess) -> Vec<Diagnostic> {
        let count = node.subtree().map(|s| s.find_all(self.tag).len()).unwrap_or(0);
        std::hint::black_box(count);
        vec![]
    }
}

struct SubtreeDeepTraversal;
impl Rule for SubtreeDeepTraversal {
    fn name(&self) -> &str { "subtree_deep" }
    fn access_mode(&self) -> AccessMode { AccessMode::CaptureSubtree }
    fn evaluate(&self, node: &dyn NodeAccess) -> Vec<Diagnostic> {
        if let Some(st) = node.subtree() {
            let mut total_attrs = 0usize;
            let mut total_text_len = 0usize;
            for desc in st.descendants() {
                total_attrs += desc.attrs.len();
                total_text_len += desc.text.len();
            }
            std::hint::black_box((total_attrs, total_text_len));
        }
        vec![]
    }
}

// ---------------------------------------------------------------------------
// XML generators
// ---------------------------------------------------------------------------

fn generate_flat_xml(num_children: usize) -> Vec<u8> {
    let mut xml = String::with_capacity(num_children * 60 + 100);
    xml.push_str(r#"<root version="1">"#);
    for i in 0..num_children {
        xml.push_str(&format!(
            r#"<item id="{}" status="active">text content {}</item>"#,
            i, i
        ));
    }
    xml.push_str("</root>");
    xml.into_bytes()
}

fn generate_deep_xml(depth: usize, children_per_level: usize) -> Vec<u8> {
    let mut xml = String::with_capacity(depth * children_per_level * 80);
    let tags: Vec<String> = (0..depth).map(|d| format!("level{}", d)).collect();

    fn build(xml: &mut String, tags: &[String], children_per_level: usize, current: usize) {
        if current >= tags.len() {
            return;
        }
        for i in 0..children_per_level {
            xml.push_str(&format!(
                r#"<{} id="{}" pos="{}">"#,
                tags[current], i, current
            ));
            xml.push_str(&format!("text at depth {} child {}", current, i));
            build(xml, tags, children_per_level, current + 1);
            xml.push_str(&format!("</{}>", tags[current]));
        }
    }

    xml.push_str(r#"<root version="1">"#);
    build(&mut xml, &tags, children_per_level, 0);
    xml.push_str("</root>");
    xml.into_bytes()
}

fn generate_mixed_xml(num_entries: usize, schema_fields: usize) -> Vec<u8> {
    let mut xml = String::with_capacity(num_entries * 80 + schema_fields * 60 + 200);
    xml.push_str(r#"<catalog version="3">"#);
    xml.push_str("<schema>");
    for i in 0..schema_fields {
        xml.push_str(&format!(
            r#"<field name="f{}" type="string" required="true"/>"#,
            i
        ));
    }
    xml.push_str("</schema>");
    for i in 0..num_entries {
        xml.push_str(&format!(
            r#"<entry sku="SKU{}" price="29.99">description {}</entry>"#,
            i, i
        ));
    }
    xml.push_str("</catalog>");
    xml.into_bytes()
}

fn generate_wide_xml(children: usize, attrs_per_child: usize) -> Vec<u8> {
    let mut xml = String::with_capacity(children * (attrs_per_child * 30 + 40) + 100);
    xml.push_str(r#"<root>"#);
    for i in 0..children {
        xml.push('<');
        xml.push_str("item");
        for a in 0..attrs_per_child {
            xml.push_str(&format!(r#" a{}="v{}_{}" "#, a, a, i));
        }
        xml.push('>');
        xml.push_str(&format!("content {}", i));
        xml.push_str("</item>");
    }
    xml.push_str("</root>");
    xml.into_bytes()
}

fn generate_noise_xml(matched: usize, unmatched_per_matched: usize) -> Vec<u8> {
    let mut xml = String::with_capacity(matched * (unmatched_per_matched + 1) * 80 + 100);
    xml.push_str(r#"<root>"#);
    for i in 0..matched {
        for j in 0..unmatched_per_matched {
            xml.push_str(&format!(
                r#"<noise_{} x="{}"><inner>junk</inner></noise_{}>"#,
                j, i, j
            ));
        }
        xml.push_str(&format!(r#"<target id="{}">hit</target>"#, i));
    }
    xml.push_str("</root>");
    xml.into_bytes()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn drain_channel(rx: &crossbeam_channel::Receiver<Diagnostic>) -> usize {
    let mut count = 0;
    while rx.try_recv().is_ok() {
        count += 1;
    }
    count
}

fn make_file(name: &str, tree: &Arc<xml_oxydizer::tree::descriptor::DescriptorTree>, xml: Vec<u8>) -> FileInfo {
    let tree = Arc::clone(tree);
    FileInfo {
        filename: name.to_owned(),
        descriptors: tree,
        stream_factory: Box::new(move || Box::new(Cursor::new(xml)) as Box<dyn std::io::Read + Send>),
    }
}

// ---------------------------------------------------------------------------
// Benchmark groups
// ---------------------------------------------------------------------------

fn bench_flat_streaming(c: &mut Criterion) {
    let mut group = c.benchmark_group("flat_streaming");
    group.sample_size(30);

    for &count in &[1_000, 10_000, 100_000, 500_000] {
        let xml = generate_flat_xml(count);
        let size = xml.len() as u64;

        let tree = Arc::new(
            TreeBuilder::new("root")
                .streaming()
                .rule(Box::new(CheckAttrRule { attr: "version", expected: "1" }))
                .node("item")
                    .streaming()
                    .rule(Box::new(CheckAttrRule { attr: "status", expected: "active" }))
                    .done()
                .build()
                .unwrap(),
        );

        group.throughput(Throughput::Bytes(size));
        group.bench_with_input(
            BenchmarkId::new("elements", count),
            &(xml, tree),
            |b, (xml, tree)| {
                b.iter(|| {
                    let (tx, rx) = bounded(4096);
                    let errors = run_pipeline(
                        vec![make_file("bench.xml", tree, xml.clone())],
                        tx,
                        &PipelineConfig::default(),
                    );
                    drain_channel(&rx);
                    assert!(errors.is_empty());
                });
            },
        );
    }
    group.finish();
}

fn bench_parse_only_no_rules(c: &mut Criterion) {
    let mut group = c.benchmark_group("parse_only");
    group.sample_size(30);

    for &count in &[10_000, 100_000, 500_000] {
        let xml = generate_flat_xml(count);
        let size = xml.len() as u64;

        let tree = Arc::new(
            TreeBuilder::new("root")
                .streaming()
                .node("item")
                    .streaming()
                    .rule(Box::new(NoopRule))
                    .done()
                .build()
                .unwrap(),
        );

        group.throughput(Throughput::Bytes(size));
        group.bench_with_input(
            BenchmarkId::new("elements", count),
            &(xml, tree),
            |b, (xml, tree)| {
                b.iter(|| {
                    let (tx, rx) = bounded(4096);
                    let errors = run_pipeline(
                        vec![make_file("bench.xml", tree, xml.clone())],
                        tx,
                        &PipelineConfig::default(),
                    );
                    drain_channel(&rx);
                    assert!(errors.is_empty());
                });
            },
        );
    }
    group.finish();
}

fn bench_heavy_rules(c: &mut Criterion) {
    let mut group = c.benchmark_group("heavy_rules");
    group.sample_size(20);

    for &count in &[10_000, 100_000] {
        let xml = generate_wide_xml(count, 10);
        let size = xml.len() as u64;

        let tree = Arc::new(
            TreeBuilder::new("root")
                .streaming()
                .node("item")
                    .streaming()
                    .rule(Box::new(HeavyAttrRule))
                    .rule(Box::new(HeavyAttrRule))
                    .rule(Box::new(HeavyAttrRule))
                    .done()
                .build()
                .unwrap(),
        );

        group.throughput(Throughput::Bytes(size));
        group.bench_with_input(
            BenchmarkId::new("elements", count),
            &(xml, tree),
            |b, (xml, tree)| {
                b.iter(|| {
                    let (tx, rx) = bounded(4096);
                    let errors = run_pipeline(
                        vec![make_file("bench.xml", tree, xml.clone())],
                        tx,
                        &PipelineConfig::default(),
                    );
                    drain_channel(&rx);
                    assert!(errors.is_empty());
                });
            },
        );
    }
    group.finish();
}

fn bench_deep_nesting(c: &mut Criterion) {
    let mut group = c.benchmark_group("deep_nesting");
    group.sample_size(20);

    for &(depth, children) in &[(5, 5), (10, 3), (15, 2)] {
        let xml = generate_deep_xml(depth, children);
        let size = xml.len() as u64;

        let tree = Arc::new(
            TreeBuilder::new("root")
                .streaming()
                .rule(Box::new(NoopRule))
                .build()
                .unwrap(),
        );

        group.throughput(Throughput::Bytes(size));
        group.bench_with_input(
            BenchmarkId::new(format!("d{}_c{}", depth, children), size),
            &(xml, tree),
            |b, (xml, tree)| {
                b.iter(|| {
                    let (tx, rx) = bounded(4096);
                    let errors = run_pipeline(
                        vec![make_file("bench.xml", tree, xml.clone())],
                        tx,
                        &PipelineConfig::default(),
                    );
                    drain_channel(&rx);
                    assert!(errors.is_empty());
                });
            },
        );
    }
    group.finish();
}

fn bench_capture_subtree(c: &mut Criterion) {
    let mut group = c.benchmark_group("capture_subtree");
    group.sample_size(20);

    for &fields in &[50, 200, 1_000] {
        let xml = generate_mixed_xml(1_000, fields);
        let size = xml.len() as u64;

        let tree = Arc::new(
            TreeBuilder::new("catalog")
                .streaming()
                .node("schema")
                    .capture_subtree()
                    .rule(Box::new(SubtreeCountRule { tag: "field" }))
                    .rule(Box::new(SubtreeDeepTraversal))
                    .done()
                .node("entry")
                    .streaming()
                    .rule(Box::new(CheckAttrRule { attr: "sku", expected: "SKU0" }))
                    .done()
                .build()
                .unwrap(),
        );

        group.throughput(Throughput::Bytes(size));
        group.bench_with_input(
            BenchmarkId::new("schema_fields", fields),
            &(xml, tree),
            |b, (xml, tree)| {
                b.iter(|| {
                    let (tx, rx) = bounded(4096);
                    let errors = run_pipeline(
                        vec![make_file("bench.xml", tree, xml.clone())],
                        tx,
                        &PipelineConfig::default(),
                    );
                    drain_channel(&rx);
                    assert!(errors.is_empty());
                });
            },
        );
    }
    group.finish();
}

fn bench_noise_skip(c: &mut Criterion) {
    let mut group = c.benchmark_group("noise_skip");
    group.sample_size(20);

    for &(matched, noise_ratio) in &[(1_000, 0), (1_000, 5), (1_000, 20)] {
        let xml = generate_noise_xml(matched, noise_ratio);
        let size = xml.len() as u64;

        let tree = Arc::new(
            TreeBuilder::new("root")
                .streaming()
                .node("target")
                    .streaming()
                    .rule(Box::new(CheckAttrRule { attr: "id", expected: "0" }))
                    .done()
                .build()
                .unwrap(),
        );

        group.throughput(Throughput::Bytes(size));
        group.bench_with_input(
            BenchmarkId::new(format!("ratio_1:{}", noise_ratio), size),
            &(xml, tree),
            |b, (xml, tree)| {
                b.iter(|| {
                    let (tx, rx) = bounded(4096);
                    let errors = run_pipeline(
                        vec![make_file("bench.xml", tree, xml.clone())],
                        tx,
                        &PipelineConfig::default(),
                    );
                    drain_channel(&rx);
                    assert!(errors.is_empty());
                });
            },
        );
    }
    group.finish();
}

fn bench_child_aggregate(c: &mut Criterion) {
    let mut group = c.benchmark_group("child_aggregate");
    group.sample_size(20);

    for &count in &[1_000, 10_000, 50_000] {
        let xml = generate_flat_xml(count);
        let size = xml.len() as u64;

        let tree = Arc::new(
            TreeBuilder::new("root")
                .streaming()
                .rule(Box::new(ChildAggregateRule))
                .node("item")
                    .streaming()
                    .rule(Box::new(NoopRule))
                    .done()
                .build()
                .unwrap(),
        );

        group.throughput(Throughput::Bytes(size));
        group.bench_with_input(
            BenchmarkId::new("children", count),
            &(xml, tree),
            |b, (xml, tree)| {
                b.iter(|| {
                    let (tx, rx) = bounded(4096);
                    let errors = run_pipeline(
                        vec![make_file("bench.xml", tree, xml.clone())],
                        tx,
                        &PipelineConfig::default(),
                    );
                    drain_channel(&rx);
                    assert!(errors.is_empty());
                });
            },
        );
    }
    group.finish();
}

fn bench_parallel_files(c: &mut Criterion) {
    let mut group = c.benchmark_group("parallel_files");
    group.sample_size(20);

    let xml_10k = generate_flat_xml(10_000);

    for &file_count in &[1, 4, 16, 64] {
        let total_bytes = xml_10k.len() as u64 * file_count as u64;

        let tree = Arc::new(
            TreeBuilder::new("root")
                .streaming()
                .rule(Box::new(CheckAttrRule { attr: "version", expected: "1" }))
                .node("item")
                    .streaming()
                    .rule(Box::new(CheckAttrRule { attr: "status", expected: "active" }))
                    .done()
                .build()
                .unwrap(),
        );

        group.throughput(Throughput::Bytes(total_bytes));
        group.bench_with_input(
            BenchmarkId::new("files", file_count),
            &(xml_10k.clone(), tree),
            |b, (xml, tree)| {
                b.iter(|| {
                    let (tx, rx) = bounded(65536);
                    let files: Vec<FileInfo> = (0..file_count)
                        .map(|i| {
                            let xml = xml.clone();
                            make_file(&format!("file_{}.xml", i), tree, xml)
                        })
                        .collect();
                    let errors = run_pipeline(files, tx, &PipelineConfig::default());
                    drain_channel(&rx);
                    assert!(errors.is_empty());
                });
            },
        );
    }
    group.finish();
}

fn bench_parallel_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("parallel_scaling");
    group.sample_size(15);

    let xml = generate_flat_xml(10_000);
    let file_count = 32;

    let tree = Arc::new(
        TreeBuilder::new("root")
            .streaming()
            .rule(Box::new(CheckAttrRule { attr: "version", expected: "1" }))
            .node("item")
                .streaming()
                .rule(Box::new(HeavyAttrRule))
                .done()
            .build()
            .unwrap(),
    );

    for &threads in &[1, 2, 4, 8] {
        let total_bytes = xml.len() as u64 * file_count as u64;
        group.throughput(Throughput::Bytes(total_bytes));
        group.bench_with_input(
            BenchmarkId::new("threads", threads),
            &(xml.clone(), tree.clone()),
            |b, (xml, tree)| {
                b.iter(|| {
                    let (tx, rx) = bounded(65536);
                    let config = PipelineConfig {
                        thread_count: Some(threads),
                        ..PipelineConfig::default()
                    };
                    let files: Vec<FileInfo> = (0..file_count)
                        .map(|i| {
                            let xml = xml.clone();
                            make_file(&format!("file_{}.xml", i), tree, xml)
                        })
                        .collect();
                    let errors = run_pipeline(files, tx, &config);
                    drain_channel(&rx);
                    assert!(errors.is_empty());
                });
            },
        );
    }
    group.finish();
}

fn bench_streaming_pipeline(c: &mut Criterion) {
    let mut group = c.benchmark_group("streaming_pipeline");
    group.sample_size(15);

    let xml = generate_flat_xml(10_000);

    let tree = Arc::new(
        TreeBuilder::new("root")
            .streaming()
            .node("item")
                .streaming()
                .rule(Box::new(CheckAttrRule { attr: "status", expected: "active" }))
                .done()
            .build()
            .unwrap(),
    );

    for &file_count in &[10, 50] {
        let total_bytes = xml.len() as u64 * file_count as u64;
        group.throughput(Throughput::Bytes(total_bytes));
        group.bench_with_input(
            BenchmarkId::new("files", file_count),
            &(xml.clone(), tree.clone()),
            |b, (xml, tree)| {
                b.iter(|| {
                    let (file_tx, file_rx) = bounded(8);
                    let (diag_tx, diag_rx) = bounded(65536);

                    let xml = xml.clone();
                    let tree = tree.clone();
                    let sender = std::thread::spawn(move || {
                        for i in 0..file_count {
                            let xml = xml.clone();
                            file_tx
                                .send(make_file(&format!("s_{}.xml", i), &tree, xml))
                                .unwrap();
                        }
                    });

                    let errors = run_pipeline_streaming(file_rx, diag_tx, &PipelineConfig::default());
                    sender.join().unwrap();
                    drain_channel(&diag_rx);
                    assert!(errors.is_empty());
                });
            },
        );
    }
    group.finish();
}

fn bench_large_single_file(c: &mut Criterion) {
    let mut group = c.benchmark_group("large_single_file");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(20));

    let xml = generate_flat_xml(1_000_000);
    let size = xml.len() as u64;

    let tree = Arc::new(
        TreeBuilder::new("root")
            .streaming()
            .node("item")
                .streaming()
                .rule(Box::new(CheckAttrRule { attr: "status", expected: "active" }))
                .done()
            .build()
            .unwrap(),
    );

    group.throughput(Throughput::Bytes(size));
    group.bench_function("1M_elements", |b| {
        b.iter(|| {
            let (tx, rx) = bounded(65536);
            let errors = run_pipeline(
                vec![make_file("big.xml", &tree, xml.clone())],
                tx,
                &PipelineConfig::default(),
            );
            drain_channel(&rx);
            assert!(errors.is_empty());
        });
    });

    group.finish();
}

fn bench_capture_large_subtree(c: &mut Criterion) {
    let mut group = c.benchmark_group("capture_large_subtree");
    group.sample_size(15);

    for &fields in &[500, 2_000, 5_000] {
        let xml = generate_mixed_xml(100, fields);
        let size = xml.len() as u64;

        let tree = Arc::new(
            TreeBuilder::new("catalog")
                .streaming()
                .node("schema")
                    .capture_subtree()
                    .rule(Box::new(SubtreeCountRule { tag: "field" }))
                    .rule(Box::new(SubtreeDeepTraversal))
                    .done()
                .node("entry")
                    .streaming()
                    .rule(Box::new(NoopRule))
                    .done()
                .build()
                .unwrap(),
        );

        group.throughput(Throughput::Bytes(size));
        group.bench_with_input(
            BenchmarkId::new("fields", fields),
            &(xml, tree),
            |b, (xml, tree)| {
                b.iter(|| {
                    let (tx, rx) = bounded(4096);
                    let errors = run_pipeline(
                        vec![make_file("bench.xml", tree, xml.clone())],
                        tx,
                        &PipelineConfig::default(),
                    );
                    drain_channel(&rx);
                    assert!(errors.is_empty());
                });
            },
        );
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_flat_streaming,
    bench_parse_only_no_rules,
    bench_heavy_rules,
    bench_deep_nesting,
    bench_capture_subtree,
    bench_noise_skip,
    bench_child_aggregate,
    bench_parallel_files,
    bench_parallel_scaling,
    bench_streaming_pipeline,
    bench_large_single_file,
    bench_capture_large_subtree,
);
criterion_main!(benches);
