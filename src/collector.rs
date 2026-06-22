use std::{collections::HashMap, error::Error, fmt::Display};
use tokio::sync::mpsc::{Receiver, Sender, error::SendError};

use crate::{cancellation::ShutdownHandle, init::FatalError, rulebuilder::RuleResult, xmlworker::FileResult};

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

impl From<CollectorError> for FatalError {
    fn from(value: CollectorError) -> Self {
        Self {
            message: format!("a fatal error occured in a collector : {:?}", value).into()
        }
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
    fatal_error_handle: ShutdownHandle<FatalError>
) -> () {
    let mut results: HashMap<u64, (String, Option<u8>, Vec<u8>, Vec<RuleResult>)> = HashMap::new();

    loop {
        tokio::select! {
            biased;
            _ = fatal_error_handle.is_cancelled() => {
                break;
            }
            file_result = collector_receiver.recv() => {
                match file_result {
                    Some(file_result) => {
                        match process_file_result(
                            &mut results,
                            file_result,
                            diagnostic_sender,
                            fatal_error_handle.clone()
                        ).await {
                            Ok(()) => {},
                            Err(err) => {
                                fatal_error_handle.trigger_fatal(err.into()).await;
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
}

pub async fn process_file_result(
    results: &mut HashMap<u64, (String, Option<u8>, Vec<u8>, Vec<RuleResult>)>,
    file_result: FileResult,
    diagnostic_sender: &Sender<FullDiagnostic>,
    fatal_error_handle: ShutdownHandle<FatalError>
) -> Result<(), CollectorError> {
    match file_result {
            FileResult::Progress(file_id, file_name, workload_counter, mut rule_results) => {
                let mut completed = false;
                match results.get_mut(&file_id) {
                    Some((_file_name, total_workload_count, workload_counters, file_rule_results)) => {
                        file_rule_results.append(&mut rule_results);
                        workload_counters.push(workload_counter);
                        workload_counters.sort();
                        completed = check_completion(
                            diagnostic_sender,
                            fatal_error_handle,
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
                            fatal_error_handle,
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
            },
            FileResult::Aborted(file_id, reason, file_name, total_workload_count_received) => {

                let mut completed = false;
                
                match results.get_mut(&file_id) {
                    None => {
                        let mut file_rule_results: Vec<RuleResult> = Vec::new();
                        let workload_counters: Vec<u8> = Vec::new();
                        file_rule_results.push(
                            RuleResult(
                                "aborted treatment".into(),
                                "root".into(),
                                false,
                                reason
                            )
                        );
                        results.insert(file_id, (file_name, Some(total_workload_count_received), workload_counters, file_rule_results));
                    },
                    Some((_file_name, total_workload_count, workload_counters, file_rule_results)) => {
                        total_workload_count.replace(total_workload_count_received.clone());
                        file_rule_results.push(
                            RuleResult(
                                "aborted treatment".into(),
                                "root".into(),
                                false,
                                reason
                            )
                        );
                        completed = check_completion(
                            diagnostic_sender,
                            fatal_error_handle,
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
            },
            FileResult::Missed(file_id, missed_paths, file_name, total_workload_count_received) => {

                let mut completed = false;
                
                match results.get_mut(&file_id) {
                    None => {
                        let mut file_rule_results: Vec<RuleResult> = Vec::new();
                        let workload_counters: Vec<u8> = Vec::new();
                        for missed_path in missed_paths.iter() {
                            file_rule_results.push(
                                RuleResult(
                                    "missed path".into(),
                                    missed_path.iter()
                                    .fold("".into(), |acc, path| { format!("{}/{}", acc, path.0) }),
                                    false,
                                    "path has not been traversed".into()
                                )
                            );
                        }
                        results.insert(file_id, (file_name, Some(total_workload_count_received), workload_counters, file_rule_results));
                    },
                    Some((_file_name, total_workload_count, workload_counters, file_rule_results)) => {
                        total_workload_count.replace(total_workload_count_received.clone());
                        for missed_path in missed_paths.iter() {
                            file_rule_results.push(
                                RuleResult(
                                    "missed path".into(),
                                    missed_path.iter()
                                    .fold("".into(), |acc, path| { format!("{}/{}", acc, path.0) }),
                                    false,
                                    "path has not been traversed".into()
                                )
                            );
                        }
                        completed = check_completion(
                            diagnostic_sender,
                            fatal_error_handle,
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
        Ok(())
}

pub async fn check_completion(
    diagnostic_sender: &Sender<FullDiagnostic>,
    fatal_error_handle: ShutdownHandle<FatalError>,
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
                fatal_error_handle.trigger_fatal(err.into()).await
            }
        }
    }
    Ok(completed)
}