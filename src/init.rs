use std::{error::Error, fmt::Display, sync::{Arc, atomic::AtomicU64}};
use educe::Educe;
use futures::future::join_all;
use itertools::Itertools;
use tokio::{io::AsyncRead, spawn, sync::{Mutex, mpsc::{self, Receiver, Sender}}, task::JoinHandle};
use std::io::Error as IoError;

use crate::{cancellation::ShutdownHandle, collector::{FullDiagnostic, collect_results}, filereader::{XmlWorkload, read}, rulebuilder::Tree, xmlworker::{FileResult, consume_xml_workload}};

#[derive(Debug, Clone)]
pub struct FatalError {
    pub message: String
}

impl FatalError {
    pub fn new(message: &str) -> Self {
        Self {
            message: message.into()
        }
    }
}

impl Display for FatalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "fatal error: {}", self.message)
    }
}

impl Error for FatalError {}

#[derive(Debug)]
pub struct ValidatorError(String);

impl Display for ValidatorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "error: {}", self.0)
    }
}

impl From<IoError> for ValidatorError {
    fn from(value: IoError) -> Self {
        ValidatorError(value.to_string())
    }
}

#[derive(Educe)]
#[educe(Debug)]
pub struct FileInfo<S>
where
    S: AsyncRead + Unpin
{
    filename: String,
    descriptors: Arc<Tree>,
    #[educe(Debug(ignore))]
    stream_factory: Box<dyn FnOnce() -> S + Send>

}

impl <S> FileInfo<S>
where
    S: AsyncRead + Unpin
{
    pub fn new(
        filename: &str,
        descriptors: Arc<Tree>,
        stream_factory: Box<dyn FnOnce() -> S + Send>

    ) -> Self {
        Self {
            filename: filename.into(),
            descriptors,
            stream_factory
        }
    }
}

pub async fn start<S>(
    file_receiver: Receiver<FileInfo<S>>,
    diagnostic_sender: &Sender<FullDiagnostic>,
    reader_count: usize,
    worker_queue_size: usize,
    worker_task_multiplier: usize,
    view_queue_size: usize,
    collector_queue_size: usize
) -> Result<(), ValidatorError>
where
    S: AsyncRead + Unpin + Send + 'static
{

    let fatal_error_handle: ShutdownHandle<FatalError> = ShutdownHandle::new();

    let global_file_id_seq = Arc::new(AtomicU64::new(0));

    // collector workers setup
    let (
        collector_senders,
        collector_receivers
    ): (
        Vec<Sender<FileResult>>,
        Vec<Receiver<FileResult>>
    ) = (1..=reader_count)
    .map(|_| mpsc::channel::<FileResult>(collector_queue_size))
    .unzip();
    let collector_handles: Vec<JoinHandle<()>> = collector_receivers.into_iter()
    .map(|mut rx| {
        let cloned_diagnostic_sender = diagnostic_sender.clone();
        let cloned_fatal_error_handle = fatal_error_handle.clone();
        spawn( async move {
            collect_results(
                &mut rx,
                &cloned_diagnostic_sender,
                cloned_fatal_error_handle
            ).await;
        })
    }).collect();
    // rule workers setup
    let (
        worker_senders,
        worker_receivers
    ): (
        Vec<Sender<XmlWorkload>>,
        Vec<Receiver<XmlWorkload>>
    ) = (1..=reader_count * worker_task_multiplier)
    .map(|_| mpsc::channel::<XmlWorkload>(worker_queue_size))
    .unzip();
    let worker_handles: Vec<JoinHandle<()>> = worker_receivers.into_iter()
    // we want a uniform distribution, so we are cycling iterator, and capping it
    .zip(collector_senders.clone().into_iter().cycle().take(reader_count * worker_task_multiplier))
    .map(|(mut rx, sender)| {
        let cloned_fatal_error_handle = fatal_error_handle.clone();
        spawn( async move {
            match consume_xml_workload(
                &mut rx,
                Arc::new(sender),
                cloned_fatal_error_handle,
                view_queue_size
            ).await {
                Ok(()) => {},
                Err(err) => {
                    println!("an error occured : {:?}", err)
                }
            }
        })
    }).collect();

    // reader setup
    let rx = Arc::new(Mutex::new(file_receiver));
    let reader_handles: Vec<JoinHandle<()>> = (1..=reader_count)
    .zip(collector_senders.into_iter())
    .zip(worker_senders.into_iter().chunks(worker_task_multiplier).into_iter())
    .map(|((_index, collector_sender), worker_senders)| {
        let rx = Arc::clone(&rx);
        let mut sender_loop = worker_senders.collect::<Vec<Sender<XmlWorkload>>>().into_iter().cycle();
        let cloned_counter = Arc::clone(&global_file_id_seq);
        let cloned_fatal_error_handle = fatal_error_handle.clone();
        spawn(async move {
            loop {
                tokio::select! {
                    biased;
                    _ = cloned_fatal_error_handle.is_cancelled() => {
                        break;
                    }
                    mut item = rx.lock() => {
                        let item = item.recv().await;
                        match sender_loop.next() {
                            Some(current_sender) => {
                                match item {
                                    Some(FileInfo {
                                        filename,
                                        descriptors,
                                        stream_factory
                                    }) => {
                                        match read(
                                        cloned_counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1,
                                        &filename,
                                        stream_factory(),
                                        descriptors,
                                        &current_sender,
                                        &collector_sender,
                                        view_queue_size,
                                        cloned_fatal_error_handle.clone()
                                    ).await {
                                        Ok(()) => {},
                                            Err(err) => { println!("an error occured : {:?}", err) } 
                                        }
                                    },
                                    None => break,
                                }
                            },
                            None => {}
                        }
                    }
            
                }
            }
        })
    }).collect();
    
    join_all(reader_handles.into_iter().chain(worker_handles.into_iter()).chain(collector_handles.into_iter())).await;

    Ok(())
}