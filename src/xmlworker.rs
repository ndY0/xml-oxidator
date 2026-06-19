use std::{collections::HashMap, fmt::Display, sync::{Arc, atomic::{AtomicBool, AtomicU64}}, time::Duration};

use tokio::{spawn, sync::{Mutex, Notify, mpsc::{self, Receiver, Sender, error::SendError}}, task::JoinHandle, time::sleep};

use crate::{filereader::XmlWorkload, rulebuilder::RuleResult};

static DEADLOCK_CHECK_INTERVAL_MS: u64 = 100;
static DEADLOCK_PURGE_INTERVAL_MS: u64 = 500;

#[derive(Debug)]
pub enum FileResult {
    Progress(u64, String, u8, Vec<RuleResult>),
    Terminated(u64, String, u8)
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

pub async fn consume_xml_workload(
    workload_receiver: &mut Receiver<XmlWorkload>,
    collector_sender: Arc<Sender<FileResult>>,
    progress: Arc<AtomicU64>,
    view_queue_size: usize,
) -> Result<(), ConsumerError> {
    let awaiting_payloads: Arc<Mutex<Vec<XmlWorkload>>> = Arc::new(Mutex::new(Vec::new()));
    let is_purging = Arc::new(AtomicBool::new(false));
    let purge_notifier = Arc::new(Notify::new());

    let (task_sender, mut task_receiver) = mpsc::channel::<JoinHandle<()>>(view_queue_size);
    let task_sender = Arc::new(task_sender);
    spawn(async move {
        while let Some(payload) = task_receiver.recv().await {
            match payload.await {
                Ok(()) => {},
                Err(err) => {
                    println!("an error occured await the rule runnning : {}", err);
                }
            }
        }
    });

    // we need a subprocess that checks at regulat interval that no pending workload had to be
    // set aside
    let checker_progress_ref = Arc::clone(&progress);
    let checker_awaiting_payloads_ref = Arc::clone(&awaiting_payloads);
    let checker_task_sender = Arc::clone(&task_sender);
    let checker_collector_sender = Arc::clone(&collector_sender);
    let checker_is_purging = Arc::clone(&is_purging);
    let checker_purge_notifier = Arc::clone(&purge_notifier);
    spawn(async move {
        loop {
            sleep(Duration::from_millis(DEADLOCK_PURGE_INTERVAL_MS)).await;
            {
                let mut awaiting_payloads = checker_awaiting_payloads_ref.lock().await;
                // dbg!(&awaiting_payloads);
                if awaiting_payloads.len() > 0 {
                    // we take precedence, no concurrency on writting to worker
                    checker_is_purging.store(true, std::sync::atomic::Ordering::Relaxed);
                    while awaiting_payloads.len() > 0 && !check_deadlock(&checker_task_sender, Arc::clone(&checker_progress_ref)).await {
                        let payload = awaiting_payloads.pop();
                        match payload {
                            Some(payload) => {
                                consume_payload(
                                &checker_task_sender,
                                payload,
                                Arc::clone(&checker_collector_sender),
                                Arc::clone(&checker_progress_ref)
                            ).await;
                            },
                            None => {}
                        }
                    }
                    checker_is_purging.store(false, std::sync::atomic::Ordering::Relaxed);
                    // storing the notification is crucial : 
                    // it ensures that we do not fall in a concurrency trap where
                    // the reader is in the process of subscribing, but we happen to notify to early.
                    // this way, it cannot miss
                    checker_purge_notifier.notify_one();
            }
            }
        }
    });
    let workload_progress_ref = Arc::clone(&progress);
    let workload_task_sender = Arc::clone(&task_sender);
    let workload_collector_sender = Arc::clone(&collector_sender);
    let workload_is_purging = Arc::clone(&is_purging);
    let workload_purge_notifier = Arc::clone(&purge_notifier);
    while let Some(payload) = workload_receiver.recv().await {
        // si on est en train de purger, patienter.
        if workload_is_purging.load(std::sync::atomic::Ordering::Relaxed) {
            dbg!("purging");
            workload_purge_notifier.notified().await;
        }
        if check_deadlock(&workload_task_sender, Arc::clone(&workload_progress_ref)).await {
            // deadlock détecté, on stock la payload sur une pile
            let mut awaiting_payloads = awaiting_payloads.lock().await;
            dbg!("pushing");
            awaiting_payloads.push(payload);
        } else {
            dbg!(workload_task_sender.capacity());
            // pas de deadlock, on peut plannifier la consommation de la payload normalement
            consume_payload(
                &workload_task_sender,
                payload,
                Arc::clone(&workload_collector_sender),
                Arc::clone(&workload_progress_ref)
            ).await;
        }
    }
    Ok(())
}

async fn check_deadlock(task_sender: &Sender<JoinHandle<()>>, progress: Arc<AtomicU64>) -> bool {
    
    // si la file est remplie
    if task_sender.capacity() == 0 {
        // we check progress a first time
        let work_progress = progress.load(std::sync::atomic::Ordering::Relaxed);
        // we delay for a few milliseconds
        sleep(Duration::from_millis(DEADLOCK_CHECK_INTERVAL_MS)).await;
        // if no work has started in the intervall, we can suppose a deadlock
        return progress.load(std::sync::atomic::Ordering::Relaxed) - work_progress == 0
    }
    false
}

async fn consume_payload(
    task_sender: &Sender<JoinHandle<()>>, 
    mut payload: XmlWorkload,
    collector_sender: Arc<Sender<FileResult>>,
    progress: Arc<AtomicU64>
) {
    let cloned_collector_sender = collector_sender.clone();
    match task_sender.send(
        spawn(async move {
            let ctx = HashMap::new();
            while let Some(view) = payload.events.recv().await {
                progress.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
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
                    println!("an error occured sending to collection : {}", err);
                }
            };
        })
    ).await {
        Ok(()) => {},
        Err(err) => {
            println!("an error occured sending to worker queue : {}", err);
        } 
    }
}