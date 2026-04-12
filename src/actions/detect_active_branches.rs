//! **Detect Active Branches** — scans repos for checked-out issue branches.
//!
//! For each issue, checks every matching repo to see if the current branch
//! starts with the issue key. Produces a map of `issue_key -> repo_label`.
//!
//! # Channel messages produced
//! - [`ActionMessage::Progress`]
//! - [`ActionMessage::ActiveBranches`]

use std::collections::HashMap;
use std::path::PathBuf;

use tokio::sync::mpsc;

use super::ActionMessage;
use crate::actions::Progress;
use crate::git;

/// Per-issue data: `(issue_key, Vec<(repo_label, repo_path)>)`.
pub type IssueBranchData = (String, Vec<(String, PathBuf)>);

/// Spawn active branch detection across all issue/repo combinations.
pub fn spawn(tx: mpsc::UnboundedSender<ActionMessage>, issue_data: Vec<IssueBranchData>) {
    let total = issue_data.len();
    let _ = tx.send(ActionMessage::TaskStarted("Scanning branches"));
    let tx = tx.clone();
    tokio::spawn(async move {
        let mut active = HashMap::new();
        for (i, (issue_key, repos)) in issue_data.into_iter().enumerate() {
            let _ = tx.send(ActionMessage::Progress(Progress {
                action: "detect_active_branches",
                message: format!("Checking {issue_key}..."),
                current: i + 1,
                total,
            }));
            for (label, path) in repos {
                let Ok(branch) = git::current_branch_in(&path).await else {
                    continue;
                };
                if branch.to_lowercase().starts_with(&issue_key.to_lowercase()) {
                    active.insert(issue_key.clone(), label);
                    break;
                }
            }
        }
        let _ = tx.send(ActionMessage::TaskFinished("Scanning branches"));
        let _ = tx.send(ActionMessage::ActiveBranches(active));
    });
}
