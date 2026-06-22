use std::collections::HashMap;
use std::io::BufRead;

use bumpalo::Bump;
use crossbeam_channel::Sender;
use quick_xml::events::Event;
use quick_xml::Reader;
use smallvec::SmallVec;

use crate::diagnostic::Diagnostic;
use crate::error::ReaderError;
use crate::reader::capture::CaptureBuilder;
use crate::reader::context::{AttrPool, ContextPool, NodeContext};
use crate::tree::descriptor::{AccessMode, DescriptorTree, NodeNeeds};
use crate::tree::path::{NodeId, PathSegment, format_path};
use crate::view::{ChildSummary, RuleResult, SubtreeView};

enum ReaderMode {
    Streaming,
    Capturing {
        builder: CaptureBuilder,
        depth: usize,
        descriptor_id: NodeId,
        context_stack_index: usize,
    },
}

pub fn parse_file<R: BufRead>(
    reader: R,
    tree: &DescriptorTree,
    diagnostics_tx: &Sender<Diagnostic>,
) -> Result<(), ReaderError> {
    let mut xml_reader = Reader::from_reader(reader);
    xml_reader.config_mut().trim_text(true);

    let mut buf = Vec::with_capacity(8192);
    let mut context_stack: Vec<NodeContext> = Vec::new();
    let mut descriptor_stack: Vec<Option<NodeId>> = Vec::new();
    let mut mode = ReaderMode::Streaming;
    let mut skip_depth: usize = 0;
    let mut sibling_counters: Vec<HashMap<u64, usize>> = Vec::new();
    let mut sibling_pool: Vec<HashMap<u64, usize>> = Vec::new();
    let mut ctx_pool = ContextPool::new();
    let mut attr_pool = AttrPool::new();

    loop {
        match xml_reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref tag)) => {
                handle_start(
                    tag, tree, &mut context_stack, &mut descriptor_stack,
                    &mut mode, &mut skip_depth, &mut sibling_counters,
                    &mut sibling_pool, &mut ctx_pool, &mut attr_pool,
                )?;
            }
            Ok(Event::End(ref _tag)) => {
                handle_end(
                    tree, &mut context_stack, &mut descriptor_stack,
                    &mut mode, &mut skip_depth, &mut sibling_counters,
                    &mut sibling_pool, diagnostics_tx, &mut ctx_pool,
                )?;
            }
            Ok(Event::Text(ref text)) => {
                if skip_depth == 0 {
                    match &mut mode {
                        ReaderMode::Capturing { builder, .. } => {
                            let decoded = text.decode().map_err(quick_xml::Error::from)?;
                            if !decoded.is_empty() {
                                builder.text(&decoded)?;
                            }
                        }
                        ReaderMode::Streaming => {
                            if let Some(ctx) = context_stack.last_mut() {
                                let desc = tree.get(ctx.descriptor_id);
                                let need_text = desc.needs.contains(NodeNeeds::TEXT)
                                    || desc.summary_needs.contains(NodeNeeds::TEXT);
                                if need_text {
                                    let decoded = text.decode().map_err(quick_xml::Error::from)?;
                                    if !decoded.is_empty() {
                                        ctx.text.push_str(&decoded);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Ok(Event::Empty(ref tag)) => {
                handle_start(
                    tag, tree, &mut context_stack, &mut descriptor_stack,
                    &mut mode, &mut skip_depth, &mut sibling_counters,
                    &mut sibling_pool, &mut ctx_pool, &mut attr_pool,
                )?;
                handle_end(
                    tree, &mut context_stack, &mut descriptor_stack,
                    &mut mode, &mut skip_depth, &mut sibling_counters,
                    &mut sibling_pool, diagnostics_tx, &mut ctx_pool,
                )?;
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(e) => return Err(e.into()),
        }
        buf.clear();
    }

    Ok(())
}

#[inline]
fn parse_attributes_into(
    tag: &quick_xml::events::BytesStart<'_>,
    out: &mut Vec<(String, String)>,
) -> Result<(), ReaderError> {
    for attr in tag.attributes() {
        let attr = attr.map_err(quick_xml::Error::from)?;
        let key = std::str::from_utf8(attr.key.as_ref())?.to_owned();
        let value = std::str::from_utf8(&attr.value)?.to_owned();
        out.push((key, value));
    }
    Ok(())
}

#[inline]
fn hash_tag(tag: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in tag.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

#[allow(clippy::too_many_arguments)]
fn handle_start(
    tag: &quick_xml::events::BytesStart<'_>,
    tree: &DescriptorTree,
    context_stack: &mut Vec<NodeContext>,
    descriptor_stack: &mut Vec<Option<NodeId>>,
    mode: &mut ReaderMode,
    skip_depth: &mut usize,
    sibling_counters: &mut Vec<HashMap<u64, usize>>,
    sibling_pool: &mut Vec<HashMap<u64, usize>>,
    ctx_pool: &mut ContextPool,
    attr_pool: &mut AttrPool,
) -> Result<(), ReaderError> {
    if *skip_depth > 0 {
        *skip_depth += 1;
        return Ok(());
    }

    match mode {
        ReaderMode::Capturing { builder, depth, .. } => {
            let name_bytes = tag.name();
            let tag_name = std::str::from_utf8(name_bytes.as_ref())?;
            let mut attrs = attr_pool.acquire();
            parse_attributes_into(tag, &mut attrs)?;
            builder.start_element(tag_name, attrs)?;
            *depth += 1;
            Ok(())
        }
        ReaderMode::Streaming => {
            let name_bytes = tag.name();
            let tag_name = std::str::from_utf8(name_bytes.as_ref())?;

            let descriptor_id = if descriptor_stack.is_empty() {
                tree.root_id().filter(|&id| tree.get(id).tag.0.as_ref() == tag_name)
            } else {
                match descriptor_stack.last().unwrap() {
                    Some(parent_id) => tree.child_of(*parent_id, tag_name),
                    None => None,
                }
            };

            match descriptor_id {
                Some(id) => {
                    let tag_hash = hash_tag(tag_name);

                    let index = if let Some(counters) = sibling_counters.last_mut() {
                        let count = counters.entry(tag_hash).or_insert(0);
                        let idx = *count;
                        *count += 1;
                        idx
                    } else {
                        0
                    };

                    let desc = tree.get(id);
                    let need_attrs = desc.needs.contains(NodeNeeds::ATTRS)
                        || desc.summary_needs.contains(NodeNeeds::ATTRS);

                    let attrs = if need_attrs || desc.access_mode == AccessMode::CaptureSubtree {
                        let mut attrs = attr_pool.acquire();
                        parse_attributes_into(tag, &mut attrs)?;
                        attrs
                    } else {
                        Vec::new()
                    };

                    descriptor_stack.push(Some(id));
                    let mut sib_map = sibling_pool.pop().unwrap_or_default();
                    sib_map.clear();
                    sibling_counters.push(sib_map);

                    if desc.access_mode == AccessMode::CaptureSubtree {
                        let ctx_index = context_stack.len();
                        let path_str = format_path(&desc.full_path);
                        let mut builder = CaptureBuilder::new(tree.capture_memory_limit, path_str);
                        builder.start_element(tag_name, attrs.clone())?;
                        context_stack.push(ctx_pool.acquire(id, attrs, index));
                        *mode = ReaderMode::Capturing {
                            builder,
                            depth: 1,
                            descriptor_id: id,
                            context_stack_index: ctx_index,
                        };
                    } else {
                        context_stack.push(ctx_pool.acquire(id, attrs, index));
                    }
                }
                None => {
                    *skip_depth = 1;
                }
            }
            Ok(())
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_end(
    tree: &DescriptorTree,
    context_stack: &mut Vec<NodeContext>,
    descriptor_stack: &mut Vec<Option<NodeId>>,
    mode: &mut ReaderMode,
    skip_depth: &mut usize,
    sibling_counters: &mut Vec<HashMap<u64, usize>>,
    sibling_pool: &mut Vec<HashMap<u64, usize>>,
    diagnostics_tx: &Sender<Diagnostic>,
    ctx_pool: &mut ContextPool,
) -> Result<(), ReaderError> {
    if *skip_depth > 0 {
        *skip_depth -= 1;
        return Ok(());
    }

    match mode {
        ReaderMode::Capturing { depth, .. } => {
            *depth -= 1;
            if *depth == 0 {
                let owned_mode = std::mem::replace(mode, ReaderMode::Streaming);
                let (builder, descriptor_id, context_stack_index) = match owned_mode {
                    ReaderMode::Capturing {
                        mut builder,
                        descriptor_id,
                        context_stack_index,
                        ..
                    } => {
                        builder.end_element();
                        (builder, descriptor_id, context_stack_index)
                    }
                    _ => unreachable!(),
                };

                let arena = Bump::new();
                let subtree_root = builder.finalize(&arena);
                let descriptor = tree.get(descriptor_id);

                let ctx = &context_stack[context_stack_index];

                let subtree_view = SubtreeView {
                    root: subtree_root,
                    descriptor_id,
                    index: ctx.index,
                    parent_stack: &context_stack[..context_stack_index],
                    tree,
                };

                let mut rule_results: SmallVec<[RuleResult; 4]> =
                    SmallVec::with_capacity(descriptor.rules.len());
                for rule in &descriptor.rules {
                    let diags = rule.evaluate(&subtree_view);
                    let passed = diags.is_empty();
                    for diag in diags {
                        let _ = diagnostics_tx.send(diag);
                    }
                    rule_results.push(RuleResult { passed });
                }

                let summary_needs = descriptor.summary_needs;
                let summary = ChildSummary {
                    tag: descriptor.tag.clone(),
                    attrs: if summary_needs.contains(NodeNeeds::ATTRS) {
                        std::mem::take(&mut context_stack[context_stack_index].attrs)
                    } else {
                        Vec::new()
                    },
                    text: if summary_needs.contains(NodeNeeds::TEXT) {
                        subtree_root.text.to_owned()
                    } else {
                        String::new()
                    },
                    index: context_stack[context_stack_index].index,
                    rule_results,
                };

                let popped = context_stack.pop().unwrap();
                ctx_pool.release(popped);
                descriptor_stack.pop();
                if let Some(sib) = sibling_counters.pop() {
                    sibling_pool.push(sib);
                }

                if let Some(parent_ctx) = context_stack.last_mut() {
                    parent_ctx.children.push(summary);
                }
            } else if let ReaderMode::Capturing { builder, .. } = mode {
                builder.end_element();
            }
        }
        ReaderMode::Streaming => {
            descriptor_stack.pop();
            if let Some(sib) = sibling_counters.pop() {
                sibling_pool.push(sib);
            }

            if let Some(ctx) = context_stack.pop() {
                let descriptor = tree.get(ctx.descriptor_id);

                let view = StreamingViewOwned {
                    ctx: &ctx,
                    parent_stack: context_stack,
                    tree,
                };

                let mut rule_results: SmallVec<[RuleResult; 4]> =
                    SmallVec::with_capacity(descriptor.rules.len());
                for rule in &descriptor.rules {
                    let diags = rule.evaluate(&view);
                    let passed = diags.is_empty();
                    for diag in diags {
                        let _ = diagnostics_tx.send(diag);
                    }
                    rule_results.push(RuleResult { passed });
                }

                let summary_needs = descriptor.summary_needs;
                let summary = ChildSummary {
                    tag: descriptor.tag.clone(),
                    attrs: if summary_needs.contains(NodeNeeds::ATTRS) {
                        ctx.attrs
                    } else {
                        Vec::new()
                    },
                    text: if summary_needs.contains(NodeNeeds::TEXT) {
                        ctx.text
                    } else {
                        String::new()
                    },
                    index: ctx.index,
                    rule_results,
                };

                if let Some(parent_ctx) = context_stack.last_mut() {
                    parent_ctx.children.push(summary);
                }
            }
        }
    }
    Ok(())
}

struct StreamingViewOwned<'a> {
    ctx: &'a NodeContext,
    parent_stack: &'a [NodeContext],
    tree: &'a DescriptorTree,
}

impl crate::rule::NodeAccess for StreamingViewOwned<'_> {
    fn tag(&self) -> &str {
        self.tree.get(self.ctx.descriptor_id).tag.0.as_ref()
    }

    fn attr(&self, name: &str) -> Option<&str> {
        self.ctx.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    fn for_each_attr(&self, f: &mut dyn FnMut(&str, &str)) {
        for (k, v) in &self.ctx.attrs {
            f(k, v);
        }
    }

    fn text(&self) -> &str {
        &self.ctx.text
    }

    fn element_index(&self) -> usize {
        self.ctx.index
    }

    fn path(&self) -> &[PathSegment] {
        &self.tree.get(self.ctx.descriptor_id).full_path
    }

    fn children_summaries(&self) -> &[ChildSummary] {
        &self.ctx.children
    }

    fn subtree(&self) -> Option<&crate::view::SubtreeNode<'_>> {
        None
    }

    fn depth(&self) -> usize {
        self.parent_stack.len()
    }

    fn ancestor_attr(&self, level: usize, name: &str) -> Option<&str> {
        let idx = self.parent_stack.len().checked_sub(level + 1)?;
        self.parent_stack[idx].attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    fn ancestor_text(&self, level: usize) -> Option<&str> {
        let idx = self.parent_stack.len().checked_sub(level + 1)?;
        Some(&self.parent_stack[idx].text)
    }

    fn ancestor_children(&self, level: usize) -> Option<&[ChildSummary]> {
        let idx = self.parent_stack.len().checked_sub(level + 1)?;
        Some(&self.parent_stack[idx].children)
    }
}
