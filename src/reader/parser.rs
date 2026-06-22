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
use crate::rule::Rule;
use crate::tree::descriptor::{DescriptorTree, NodeNeeds};
use crate::tree::path::{NodeId, format_path};
use crate::view::{ChildSummary, RuleResults, SubtreeView};

enum ReaderMode {
    Streaming,
    Capturing {
        builder: CaptureBuilder,
        depth: usize,
        descriptor_id: NodeId,
        context_stack_index: usize,
    },
}

/// Default capacity for the diagnostics flush buffer.
const DIAG_BUFFER_DEFAULT: usize = 256;

pub fn parse_file<R: Rule, B: BufRead>(
    reader: B,
    tree: &DescriptorTree<R>,
    diagnostics_tx: &Sender<Diagnostic>,
) -> Result<(), ReaderError> {
    let mut xml_reader = Reader::from_reader(reader);
    xml_reader.config_mut().trim_text(true);

    let mut buf = Vec::with_capacity(8192);
    let mut context_stack: Vec<NodeContext> = Vec::new();
    let mut descriptor_stack: Vec<Option<NodeId>> = Vec::new();
    let mut mode = ReaderMode::Streaming;
    let mut skip_depth: usize = 0;
    let mut sibling_counters: Vec<SmallVec<[(u64, u32); 8]>> = Vec::new();
    let mut sibling_pool: Vec<SmallVec<[(u64, u32); 8]>> = Vec::new();
    let mut ctx_pool = ContextPool::new();
    let mut attr_pool = AttrPool::new();
    let mut diag_buffer: Vec<Diagnostic> = Vec::with_capacity(DIAG_BUFFER_DEFAULT);

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
                    &mut sibling_pool, &mut diag_buffer, diagnostics_tx,
                    &mut ctx_pool,
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
                    &mut sibling_pool, &mut diag_buffer, diagnostics_tx,
                    &mut ctx_pool,
                )?;
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(e) => return Err(e.into()),
        }
        buf.clear();
    }

    // Flush remaining diagnostics
    flush_diagnostics(&mut diag_buffer, diagnostics_tx);

    Ok(())
}

/// Zero-copy variant for in-memory XML data.
pub fn parse_slice<R: Rule>(
    data: &[u8],
    tree: &DescriptorTree<R>,
    diagnostics_tx: &Sender<Diagnostic>,
) -> Result<(), ReaderError> {
    let text = std::str::from_utf8(data)?;
    let mut xml_reader = Reader::from_str(text);
    xml_reader.config_mut().trim_text(true);

    let mut context_stack: Vec<NodeContext> = Vec::new();
    let mut descriptor_stack: Vec<Option<NodeId>> = Vec::new();
    let mut mode = ReaderMode::Streaming;
    let mut skip_depth: usize = 0;
    let mut sibling_counters: Vec<SmallVec<[(u64, u32); 8]>> = Vec::new();
    let mut sibling_pool: Vec<SmallVec<[(u64, u32); 8]>> = Vec::new();
    let mut ctx_pool = ContextPool::new();
    let mut attr_pool = AttrPool::new();
    let mut diag_buffer: Vec<Diagnostic> = Vec::with_capacity(DIAG_BUFFER_DEFAULT);

    loop {
        match xml_reader.read_event() {
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
                    &mut sibling_pool, &mut diag_buffer, diagnostics_tx,
                    &mut ctx_pool,
                )?;
            }
            Ok(Event::Text(ref text_ev)) => {
                if skip_depth == 0 {
                    match &mut mode {
                        ReaderMode::Capturing { builder, .. } => {
                            let decoded = text_ev.decode().map_err(quick_xml::Error::from)?;
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
                                    let decoded = text_ev.decode().map_err(quick_xml::Error::from)?;
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
                    &mut sibling_pool, &mut diag_buffer, diagnostics_tx,
                    &mut ctx_pool,
                )?;
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(e) => return Err(e.into()),
        }
    }

    flush_diagnostics(&mut diag_buffer, diagnostics_tx);

    Ok(())
}

#[inline(always)]
fn flush_diagnostics(buffer: &mut Vec<Diagnostic>, tx: &Sender<Diagnostic>) {
    for diag in buffer.drain(..) {
        let _ = tx.send(diag);
    }
}

#[inline(always)]
fn buffer_diagnostic(buffer: &mut Vec<Diagnostic>, tx: &Sender<Diagnostic>, diag: Diagnostic) {
    buffer.push(diag);
    if buffer.len() >= DIAG_BUFFER_DEFAULT {
        flush_diagnostics(buffer, tx);
    }
}

#[inline(always)]
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

#[inline(always)]
fn hash_tag(tag: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in tag.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Linear scan over SmallVec for sibling counter lookup/insert.
#[inline(always)]
fn sibling_counter_get_or_insert(counters: &mut SmallVec<[(u64, u32); 8]>, tag_hash: u64) -> u32 {
    for entry in counters.iter_mut() {
        if entry.0 == tag_hash {
            let idx = entry.1;
            entry.1 += 1;
            return idx;
        }
    }
    counters.push((tag_hash, 1));
    0
}

#[inline(always)]
#[allow(clippy::too_many_arguments)]
fn handle_start<R: Rule>(
    tag: &quick_xml::events::BytesStart<'_>,
    tree: &DescriptorTree<R>,
    context_stack: &mut Vec<NodeContext>,
    descriptor_stack: &mut Vec<Option<NodeId>>,
    mode: &mut ReaderMode,
    skip_depth: &mut usize,
    sibling_counters: &mut Vec<SmallVec<[(u64, u32); 8]>>,
    sibling_pool: &mut Vec<SmallVec<[(u64, u32); 8]>>,
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
                        sibling_counter_get_or_insert(counters, tag_hash)
                    } else {
                        0
                    };

                    let desc = tree.get(id);
                    let is_capture = desc.needs.is_capture();
                    let need_attrs = desc.needs.contains(NodeNeeds::ATTRS)
                        || desc.summary_needs.contains(NodeNeeds::ATTRS);

                    let attrs = if need_attrs || is_capture {
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

                    if is_capture {
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

#[inline(always)]
#[allow(clippy::too_many_arguments)]
fn handle_end<R: Rule>(
    tree: &DescriptorTree<R>,
    context_stack: &mut Vec<NodeContext>,
    descriptor_stack: &mut Vec<Option<NodeId>>,
    mode: &mut ReaderMode,
    skip_depth: &mut usize,
    sibling_counters: &mut Vec<SmallVec<[(u64, u32); 8]>>,
    sibling_pool: &mut Vec<SmallVec<[(u64, u32); 8]>>,
    diag_buffer: &mut Vec<Diagnostic>,
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

                let mut rule_results = RuleResults::empty();
                for (i, rule) in descriptor.rules.iter().enumerate() {
                    let diags = rule.evaluate(&subtree_view);
                    let passed = diags.is_empty();
                    for diag in diags {
                        buffer_diagnostic(diag_buffer, diagnostics_tx, diag);
                    }
                    rule_results.set(i, passed);
                }

                let summary_needs = descriptor.summary_needs;
                let summary = ChildSummary {
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
                    descriptor_id,
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

                let mut rule_results = RuleResults::empty();
                for (i, rule) in descriptor.rules.iter().enumerate() {
                    let diags = rule.evaluate(&view);
                    let passed = diags.is_empty();
                    for diag in diags {
                        buffer_diagnostic(diag_buffer, diagnostics_tx, diag);
                    }
                    rule_results.set(i, passed);
                }

                let summary_needs = descriptor.summary_needs;
                let summary = ChildSummary {
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
                    descriptor_id: ctx.descriptor_id,
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

struct StreamingViewOwned<'a, R> {
    ctx: &'a NodeContext,
    parent_stack: &'a [NodeContext],
    tree: &'a DescriptorTree<R>,
}

impl<R: Rule> crate::rule::NodeAccess for StreamingViewOwned<'_, R> {
    #[inline(always)]
    fn tag(&self) -> &str {
        self.tree.get(self.ctx.descriptor_id).tag.0.as_ref()
    }

    #[inline(always)]
    fn attr(&self, name: &str) -> Option<&str> {
        self.ctx.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    #[inline(always)]
    fn for_each_attr(&self, f: &mut dyn FnMut(&str, &str)) {
        for (k, v) in &self.ctx.attrs {
            f(k, v);
        }
    }

    #[inline(always)]
    fn text(&self) -> &str {
        &self.ctx.text
    }

    #[inline(always)]
    fn element_index(&self) -> usize {
        self.ctx.index as usize
    }

    #[inline(always)]
    fn path(&self) -> &[crate::tree::path::PathSegment] {
        &self.tree.get(self.ctx.descriptor_id).full_path
    }

    #[inline(always)]
    fn children_summaries(&self) -> &[ChildSummary] {
        &self.ctx.children
    }

    #[inline(always)]
    fn subtree(&self) -> Option<&crate::view::SubtreeNode<'_>> {
        None
    }

    #[inline(always)]
    fn depth(&self) -> usize {
        self.parent_stack.len()
    }

    #[inline(always)]
    fn ancestor_attr(&self, level: usize, name: &str) -> Option<&str> {
        let idx = self.parent_stack.len().checked_sub(level + 1)?;
        self.parent_stack[idx].attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    #[inline(always)]
    fn ancestor_text(&self, level: usize) -> Option<&str> {
        let idx = self.parent_stack.len().checked_sub(level + 1)?;
        Some(&self.parent_stack[idx].text)
    }

    #[inline(always)]
    fn ancestor_children(&self, level: usize) -> Option<&[ChildSummary]> {
        let idx = self.parent_stack.len().checked_sub(level + 1)?;
        Some(&self.parent_stack[idx].children)
    }
}
