use std::{cell::RefCell, fmt::Display, rc::Rc, sync::Arc};
use futures::Stream;
use itertools::Itertools;
use tokio::{runtime::Builder, sync::{Mutex, mpsc::{self, Sender, Receiver}}, task::JoinHandle};
use tokio_util::bytes::Buf;
use std::io::Error as IoError;

use crate::{collector::collect_results, filereader::{TechnicalError, XmlWorkload, read}, rulebuilder::Node, xmlworker::{FileRuleResult, consume_xml_workload}};

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

pub struct FileInfo<B, E, S>
where
    S: Stream<Item = Result<B, E>> + Unpin,
    B: Buf,
    E: Into<std::io::Error>,
{
    filename: String,
    stream_factory: Box<dyn FnOnce() -> S>

}

pub async fn start<'a, B, E, S>(
    file_receiver: Receiver<FileInfo<B, E, S>>,
    error_sender: &Sender<TechnicalError>,
    descriptors: Node<'a>,
    reader_count: usize,
    (worker_count_multiplier, worker_queue_size): (usize, usize),
    (collector_count, collector_queue_size): (usize, usize),
    reader_highwatermark: usize
) -> Result<(), ValidatorError>
where
    S: Stream<Item = Result<B, E>> + Unpin,
    B: Buf,
    E: Into<std::io::Error>,
{

    let readers_runtime = Builder::new_multi_thread()
    .worker_threads(reader_count)
    .thread_name("reader-pool")
    .build()?;

    let workers_runtime = tokio::runtime::Builder::new_multi_thread()
    .worker_threads(worker_count_multiplier * collector_count)
    .thread_name("rule-pool")
    .build()?;

    let collectors_runtime = tokio::runtime::Builder::new_multi_thread()
    .worker_threads(collector_count)
    .thread_name("collector-pool")
    .build()?;
    
    // collector workers setup
    let (
        collector_senders,
        collector_receivers
    ): (
        Vec<Sender<FileRuleResult>>,
        Vec<Receiver<FileRuleResult>>
    ) = (1..=collector_count)
    .map(|_| mpsc::channel::<FileRuleResult>(collector_queue_size))
    .unzip();
    let collector_handles: Vec<JoinHandle<()>> = collector_receivers.into_iter()
    .map(|mut rx| {
        // let (tx, mut rx) = mpsc::channel::<FileRuleResult>(collector_queue_size);
        collectors_runtime.spawn( async move {
            collect_results(&mut rx).await
        })
    }).collect();

    // rule workers setup
    let (
        worker_senders,
        worker_receivers
    ): (
        Vec<Sender<XmlWorkload>>,
        Vec<Receiver<XmlWorkload>>
    ) = (1..=worker_count_multiplier * collector_count)
    .map(|_| mpsc::channel::<XmlWorkload>(worker_queue_size))
    .unzip();
    let worker_handles: Vec<JoinHandle<()>> = worker_receivers.into_iter()
    // we want a uniform distribution, so we are cycling iterator, and capping it
    .zip(collector_senders.into_iter().cycle().take(worker_count_multiplier * collector_count))
    .map(|(mut rx, sender)| {
        workers_runtime.spawn( async move {
            loop {
                match consume_xml_workload(&mut rx, &sender).await {
                    Ok(()) => {},
                    Err(err) => {
                        println!("an error occured : {:?}", err)
                    }
                }
            }
        })
    }).collect();

    // reader setup
    // first off, we need to setup a queue for file to read
    let (tx, rx) = mpsc::channel::<>(100);
    let rx = Arc::new(Mutex::new(file_receiver));
    let reader_handles = (1..=collector_count)
    .zip(worker_senders.into_iter().chunks(worker_count_multiplier).into_iter())
    .map(|(index, worker_senders)| {
        let rx = rx.clone();
        tokio::spawn(async move {
            let sender_loop = worker_senders.collect::<Vec<Sender<XmlWorkload>>>().iter().cycle();
            loop {
                let item = { rx.lock().await.recv().await };
                match sender_loop.next() {
                    Some(current_sender) => {
                        match item {
                            Some(FileInfo {
                                filename,
                                stream_factory
                            }) => {
                                match read(
                                &filename,
                                stream_factory(),
                                Rc::new(RefCell::new(descriptors)),
                                current_sender,
                                error_sender,
                                reader_highwatermark
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
        })
    }).collect();
    for _ in 1..collector_count {
    }

    Ok(())
}