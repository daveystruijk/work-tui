//! **Detect Active Branches** — scans repos for checked-out issue branches.
//!
//! For each issue, checks every matching repo to see if the current branch
//! starts with the issue key. Produces a map of `issue_key -> repo_label` for
//! active branches, plus a set of dirty repo paths.
//!
//! # Channel messages produced
//! - [`Message::Progress`]
//! - [`Message::ActiveBranches`]

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use tokio::sync::mpsc;

use super::Message;
use crate::actions::Progress;
use crate::git;

/// Per-issue data: `(issue_key, Vec<(repo_label, repo_path)>)`.
pub type IssueBranchData = (String, Vec<(String, PathBuf)>);

/// Spawn active branch detection across all issue/repo combinations.
pub fn spawn(tx: mpsc::UnboundedSender<Message>, issue_data: Vec<IssueBranchData>) {
    super::spawn_action(
        tx,
        "detect_active_branches",
        "Scanning branches",
        |tx| async move {
            let total = issue_data.len();
            let mut active = HashMap::new();
            let mut dirty_repos = HashSet::new();
            let mut checked_paths = HashSet::new();
            for (i, (issue_key, repos)) in issue_data.into_iter().enumerate() {
                let _ = tx.send(Message::Progress(Progress {
                    task_id: "detect_active_branches".into(),
                    message: format!("Checking {issue_key}..."),
                    current: i + 1,
                    total,
                }));
                for (label, path) in repos {
                    // Check dirtiness once per unique repo path.
                    if checked_paths.insert(path.clone()) {
                        let dirty = !git::is_clean(&path).await.unwrap_or(true);
                        if dirty {
                            dirty_repos.insert(path.clone());
                        }
                    }

                    if active.contains_key(&issue_key) {
                        continue;
                    }
                    let Ok(branch) = git::current_branch_in(&path).await else {
                        continue;
                    };
                    if branch.to_lowercase().starts_with(&issue_key.to_lowercase()) {
                        active.insert(issue_key.clone(), label);
                    }
                }
            }
            let _ = tx.send(Message::ActiveBranches(active, dirty_repos));
        },
    );
}
