use std::{collections::HashMap, fmt::Display, sync::{Arc, atomic::AtomicU64}};

use tokio::{spawn, sync::mpsc::{self, Receiver, Sender, error::SendError}, task::JoinHandle};

use crate::{cancellation::ShutdownHandle, filereader::XmlWorkload, init::FatalError, rulebuilder::{Path, RuleResult}};

#[derive(Debug)]
pub enum FileResult {
    Progress(u64, String, u8, Vec<RuleResult>),
    Terminated(u64, String, u8),
    Aborted(u64, String, String, u8),
    Missed(u64, Vec<Vec<Path>>, String, u8)

}

#[derive(Debug)]
pub struct ConsumerError(String);

impl Display for ConsumerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "error: {}", self.0)
    }
}

impl <T> From<SendError<T>> for ConsumerError {
    fn from(value: SendError<T>) -> Self {
        ConsumerError(value.to_string())
    }
}

impl From<ConsumerError> for FatalError {
    fn from(value: ConsumerError) -> Self {
        Self {
            message: format!("a fatal error occured in a consumer worker : {:?}", value).into()
        }
    }
}

impl <T> From<SendError<T>> for FatalError {
    fn from(value: SendError<T>) -> Self {
        Self {
            message: format!("a fatal error occured in a consumer worker : {:?}", value).into()
        }
    }
}

pub async fn consume_xml_workload(
    workload_receiver: &mut Receiver<XmlWorkload>,
    collector_sender: Arc<Sender<FileResult>>,
    fatal_error_handle: ShutdownHandle<FatalError>,
    view_queue_size: usize,
) -> Result<(), ConsumerError> {
    let payloads_count = Arc::new(AtomicU64::new(0));
    let (task_sender, mut task_receiver) = mpsc::channel::<JoinHandle<()>>(view_queue_size);
    let task_sender = Arc::new(task_sender);
    let task_payloads_count = Arc::clone(&payloads_count);
    let task_fatal_error_handle = fatal_error_handle.clone();
    spawn(async move {
        loop {
            tokio::select! {
                biased;
                _ = task_fatal_error_handle.is_cancelled() => {
                    while let Some(task) = task_receiver.recv().await {
                        task.abort();
                    }
                    break;
                }
                payload = task_receiver.recv() => {
                    match payload {
                        Some(payload) => {
                            match payload.await {
                                Ok(()) => {
                                    task_payloads_count.fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
                                },
                                Err(err) => {
                                    println!("an error occured await the rule runnning : {}", err);
                                }
                            }
                        },
                        None => {
                            break;
                        }
                    }
                }
            }
        }
    });
    let workload_fatal_error_handle = fatal_error_handle.clone();
    loop {
        tokio::select! {
            biased;
            _ = workload_fatal_error_handle.is_cancelled() => {
                break;
            }
            payload = workload_receiver.recv() => {
                match payload {
                    Some(payload) => {
                        if
                            task_sender.capacity() == 0
                            && task_sender.max_capacity() as u64 <= payloads_count.load(std::sync::atomic::Ordering::Relaxed)
                        {
                            workload_fatal_error_handle.trigger_fatal(FatalError::new(" \
                                deadlock apppeared, it seems that you worker pool configuration is to shallow \
                                to accomodate your testing path depth. consider increasing the task multiplier \
                            ".into())).await;
                            break;
                        }
                        match consume_payload(
                            &task_sender,
                            payload,
                            Arc::clone(&collector_sender),
                            Arc::clone(&payloads_count),
                            workload_fatal_error_handle.clone()
                        ).await {
                            Ok(()) => {},
                            Err(err) => {
                                workload_fatal_error_handle.trigger_fatal(err.into()).await;
                            }
                        }
                    },
                    None => {
                        break;
                    }
                }
            }
        }
    }
    Ok(())
}

async fn consume_payload(
    task_sender: &Sender<JoinHandle<()>>, 
    mut payload: XmlWorkload,
    collector_sender: Arc<Sender<FileResult>>,
    payloads_count: Arc<AtomicU64>,
    workload_fatal_error_handle: ShutdownHandle<FatalError>
) -> Result<(), ConsumerError> {
    let cloned_collector_sender = collector_sender.clone();
    task_sender.send(
        spawn(async move {
            let ctx = HashMap::new();
            while let Some(view) = payload.events.recv().await {
                for rule in payload.rules.iter_mut() {
                    rule.fold(&view, &ctx);
                }
            }
            let results: Vec<RuleResult> = payload.rules.iter()
            .map(|rule| {
                let diagnostic = rule.assert(&payload.path.iter().fold(String::new(), |acc, curr| format!("{}/{}", acc, curr.0)));
                RuleResult(
                    payload.path.iter().fold("".into(), |acc, path| { format!("{}/{}", acc, path.0) }),
                    diagnostic.rule_name,
                    diagnostic.statut,
                    diagnostic.assertion
                )
            }).collect();
            match cloned_collector_sender.send(FileResult::Progress(payload.file_id, payload.file, payload.workload_counter, results)).await {
                Ok(()) => {},
                Err(err) => {
                    workload_fatal_error_handle.trigger_fatal(err.into()).await;
                }
            };
        })
    ).await?;
    payloads_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    Ok(())
}