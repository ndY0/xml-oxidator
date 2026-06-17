use std::{collections::HashMap, fmt::Display};

use tokio::sync::mpsc::{Receiver, Sender, error::SendError};

use crate::{filereader::XmlWorkload, rulebuilder::RuleResult};

pub enum FileResult {
    Progress(u64, String, Vec<RuleResult>),
    Terminated(u64, String)
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
    collector_sender: &Sender<FileResult>
) -> Result<(), ConsumerError> {

    while let Some(mut payload) = workload_receiver.recv().await {

        // in the futur, consider using a real context
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
        collector_sender.send(FileResult::Progress(payload.file_id, payload.file, results)).await?;
    }
    Ok(())
}