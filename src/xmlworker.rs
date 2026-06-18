use std::{collections::HashMap, fmt::Display};

use tokio::{spawn, sync::mpsc::{self, Receiver, Sender, error::SendError}, task::JoinHandle};

use crate::{filereader::XmlWorkload, rulebuilder::RuleResult};

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
    collector_sender: &Sender<FileResult>,
    view_queue_size: usize,
) -> Result<(), ConsumerError> {

    let (task_sender, mut task_receiver) = mpsc::channel::<JoinHandle<()>>(view_queue_size);
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

    while let Some(mut payload) = workload_receiver.recv().await {
        let cloned_collector_send = collector_sender.clone();
        match task_sender.send(
            spawn(async move {
                let ctx = HashMap::new();
                while let Some(view) = payload.events.recv().await {
                    for rule in payload.rules.iter_mut() {
                        rule.fold(&view, &ctx);
                    }
                }
                let results: Vec<RuleResult> = payload.rules.iter()
                .map(|rule| {
                    let diagnostic = rule.assert();
                    RuleResult(
                        payload.path.iter().fold("".into(), |acc, path| { format!("{}/{}", acc, path.0) }),
                        diagnostic.rule_name,
                        diagnostic.statut,
                        diagnostic.assertion
                    )
                }).collect();
                match cloned_collector_send.send(FileResult::Progress(payload.file_id, payload.file, payload.workload_counter, results)).await {
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
    Ok(())
}