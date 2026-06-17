use std::{collections::HashMap, error::Error, fmt::Display};
use tokio::sync::{Mutex, mpsc::{Receiver, Sender, error::SendError}};

use crate::{filereader::TechnicalError, rulebuilder::RuleResult, xmlworker::FileResult};

#[derive(Debug)]
pub struct CollectorError(String);

impl Display for CollectorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "error: {}", self.0)
    }
}
impl Error for CollectorError {}

impl <T> From<SendError<T>> for CollectorError {
    fn from(value: SendError<T>) -> Self {
        CollectorError(value.to_string())
    }
}

#[derive(Debug)]
pub struct FullDiagnostic {
    pub filename: String,
    pub diagnotics: Vec<RuleDiagnostic>
}

#[derive(Debug)]
pub struct RuleDiagnostic {
    pub rule_name: String,
    pub path: String,
    pub status: bool,
    pub comment: String
}

impl From<(String, Vec<RuleResult>)> for FullDiagnostic {
    fn from((file_name, rule_results): (String, Vec<RuleResult>)) -> Self {
        FullDiagnostic {
            filename: file_name,
            diagnotics: rule_results.iter()
            .map(|RuleResult(rule_name, path, status, comment)| {
                RuleDiagnostic {
                    rule_name: rule_name.clone(),
                    path: path.clone(),
                    status: *status,
                    comment: comment.clone()
                }
            }).collect()
        }
    }
}

pub async fn collect_results(
    collector_receiver: &mut Receiver<FileResult>,
    diagnostic_sender: &Sender<FullDiagnostic>,
    error_sender: &Sender<TechnicalError>,
) -> Result<(), CollectorError> {
    let results: Mutex<HashMap<u64, (String, Vec<RuleResult>)>> = Mutex::new(HashMap::new());
    while let Some(file_results) = collector_receiver.recv().await {
        let mut results = results.lock().await;
        match file_results {
            FileResult::Progress(file_id, file_name, mut rule_results) => {
                dbg!(&rule_results);
                match results.get_mut(&file_id) {
                    Some((_file_name, file_rule_results)) => {
                        file_rule_results.append(&mut rule_results);
                    },
                    None => {
                        let mut file_rule_results: Vec<RuleResult> = Vec::new();
                        file_rule_results.append(&mut rule_results);
                        results.insert(file_id, (file_name, file_rule_results));
                    }
                }
            },
            FileResult::Terminated(file_id, file_name) => {
                dbg!(&file_name);
                // TODO: add a counter for payload tracking and actual completion
                // match results.remove(&file_id) {
                //     Some((file_name, file_rule_results)) => {
                //         match diagnostic_sender.send((file_name, file_rule_results).into()).await {
                //             Ok(_) => {},
                //             Err(err) => {
                //                 error_sender.send(TechnicalError {
                //                     error: err.to_string(),
                //                     file: file_id.to_string()
                //                 }).await?
                //             }
                //         }
                //     },
                //     None => {
                //         match diagnostic_sender.send((file_name, vec![]).into()).await {
                //             Ok(_) => {},
                //             Err(err) => {
                //                 error_sender.send(TechnicalError {
                //                     error: err.to_string(),
                //                     file: file_id.to_string()
                //                 }).await?
                //             }
                //         }
                //     }
                // }
            }
        }
    }
    Ok(())
}