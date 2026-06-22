use std::io::{BufReader, Read};
use std::sync::Arc;

use crossbeam_channel::{Receiver, Sender};
use rayon::prelude::*;

use crate::diagnostic::Diagnostic;
use crate::error::PipelineError;
use crate::reader::parser::parse_file;
use crate::rule::Rule;
use crate::tree::descriptor::DescriptorTree;

pub struct FileInfo<R> {
    pub filename: String,
    pub descriptors: Arc<DescriptorTree<R>>,
    pub stream_factory: Box<dyn FnOnce() -> Box<dyn Read + Send> + Send>,
}

impl<R> std::fmt::Debug for FileInfo<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileInfo")
            .field("filename", &self.filename)
            .finish_non_exhaustive()
    }
}

pub struct PipelineConfig {
    pub thread_count: Option<usize>,
    pub buf_reader_capacity: usize,
    pub diagnostics_buffer_size: usize,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            thread_count: None,
            buf_reader_capacity: 64 * 1024,
            diagnostics_buffer_size: 256,
        }
    }
}

pub fn run_pipeline<R: Rule + 'static>(
    files: Vec<FileInfo<R>>,
    diagnostics_tx: Sender<Diagnostic>,
    config: &PipelineConfig,
) -> Vec<PipelineError> {
    let pool = build_pool(config);
    pool.install(|| {
        files
            .into_par_iter()
            .filter_map(|file| process_file(file, &diagnostics_tx, config.buf_reader_capacity))
            .collect()
    })
}

pub fn run_pipeline_streaming<R: Rule + 'static>(
    file_rx: Receiver<FileInfo<R>>,
    diagnostics_tx: Sender<Diagnostic>,
    config: &PipelineConfig,
) -> Vec<PipelineError> {
    let pool = build_pool(config);
    pool.install(|| {
        file_rx
            .into_iter()
            .par_bridge()
            .filter_map(|file| process_file(file, &diagnostics_tx, config.buf_reader_capacity))
            .collect()
    })
}

fn build_pool(config: &PipelineConfig) -> rayon::ThreadPool {
    let mut builder = rayon::ThreadPoolBuilder::new();
    if let Some(threads) = config.thread_count {
        builder = builder.num_threads(threads);
    }
    builder.build().expect("failed to build rayon thread pool")
}

fn process_file<R: Rule>(
    file_info: FileInfo<R>,
    diagnostics_tx: &Sender<Diagnostic>,
    buf_reader_capacity: usize,
) -> Option<PipelineError> {
    let FileInfo {
        filename,
        descriptors,
        stream_factory,
    } = file_info;
    let reader = stream_factory();
    let buf_reader = BufReader::with_capacity(buf_reader_capacity, reader);

    match parse_file(buf_reader, &descriptors, diagnostics_tx) {
        Ok(()) => None,
        Err(e) => Some(PipelineError::Reader {
            filename,
            source: e,
        }),
    }
}
