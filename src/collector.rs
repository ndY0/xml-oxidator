use std::{collections::HashMap, io::Write};

use tokio::sync::{Mutex, mpsc::Receiver};

use crate::xmlworker::FileRuleResult;

pub async fn collect_results<W: Write>(
    collector_sender: &mut Receiver<FileRuleResult>,
    output: W
) {
    let results: Mutex<HashMap<String, Vec<FileRuleResult>>> = Mutex::new(HashMap::new());
    while let Some(file_results) = collector_sender.recv().await {
        let mut results = results.lock().await;
        match results.get_mut(&file_results.file) {
            Some(file_rule_results) => {
                file_rule_results.push(file_results);
            },
            None => {
                let file = file_results.file.clone();
                let mut file_rule_results: Vec<FileRuleResult> = Vec::new();
                file_rule_results.push(file_results);
                results.insert(file, file_rule_results);

            }
        }
    }
}