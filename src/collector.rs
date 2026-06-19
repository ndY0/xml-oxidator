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

impl From<(String, &mut Vec<RuleResult>)> for FullDiagnostic {
    fn from((file_name, rule_results): (String, &mut Vec<RuleResult>)) -> Self {
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
    let results: Mutex<HashMap<u64, (String, Option<u8>, Vec<u8>, Vec<RuleResult>)>> = Mutex::new(HashMap::new());
    while let Some(file_results) = collector_receiver.recv().await {
        let mut results = results.lock().await;
        match file_results {
            FileResult::Progress(file_id, file_name, workload_counter, mut rule_results) => {
                let mut completed = false;
                match results.get_mut(&file_id) {
                    Some((_file_name, total_workload_count, workload_counters, file_rule_results)) => {
                        file_rule_results.append(&mut rule_results);
                        workload_counters.push(workload_counter);
                        workload_counters.sort();
                        completed = check_completion(
                            diagnostic_sender,
                            error_sender,
                            file_id,
                            file_name,
                            file_rule_results,
                            workload_counters,
                            total_workload_count
                        ).await?;
                    },
                    None => {
                        let mut file_rule_results: Vec<RuleResult> = Vec::new();
                        let mut workload_counters: Vec<u8> = Vec::new();
                        file_rule_results.append(&mut rule_results);
                        workload_counters.push(workload_counter);
                        results.insert(file_id, (file_name, None, workload_counters, file_rule_results));
                    }
                }
                if completed {
                    results.remove(&file_id);
                }
            },
            FileResult::Terminated(file_id, file_name, total_workload_count_received) => {
                let mut completed = false;
                
                match results.get_mut(&file_id) {
                    None => {
                        let file_rule_results: Vec<RuleResult> = Vec::new();
                        let workload_counters: Vec<u8> = Vec::new();
                        results.insert(file_id, (file_name, Some(total_workload_count_received), workload_counters, file_rule_results));
                    },
                    Some((_file_name, total_workload_count, workload_counters, file_rule_results)) => {
                        total_workload_count.replace(total_workload_count_received.clone());
                        completed = check_completion(
                            diagnostic_sender,
                            error_sender,
                            file_id,
                            file_name,
                            file_rule_results,
                            workload_counters,
                            total_workload_count
                        ).await?;
                    }
                }
                if completed {
                    results.remove(&file_id);
                }
            }
        }
    }
    Ok(())
}

pub async fn check_completion(
    diagnostic_sender: &Sender<FullDiagnostic>,
    error_sender: &Sender<TechnicalError>,
    file_id: u64,
    file_name: String,
    results: &mut Vec<RuleResult>,
    workload_counters: &mut Vec<u8>,
    total_workload_count: &mut Option<u8>,

) -> Result<bool, CollectorError> {
    let completed = total_workload_count
    .is_some_and(
        |total| workload_counters
        .get(workload_counters.len() - 1)
        .is_some_and(|max| *max == total)
    )
    && workload_counters.iter().zip(0u8..=(workload_counters.len() as u8 - 1))
    .fold(true, |acc, (count, cmp)| acc && *count == cmp);

    if completed {
        match diagnostic_sender.send((file_name, results).into()).await {
            Ok(_) => {},
            Err(err) => {
                error_sender.send(TechnicalError {
                    error: err.to_string(),
                    file: file_id.to_string()
                }).await?
            }
        }
    }

    Ok(completed)
}