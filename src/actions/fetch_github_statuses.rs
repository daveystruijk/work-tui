//! **Fetch GitHub Statuses** — resolves CI/PR status for each issue.
//!
//! Iterates over issues that have a matching repo and calls `gh pr list --head`
//! to find the associated PR and its check status. Results are sent one at a
//! time so the UI can show incremental progress.
//!
//! # Channel messages produced
//! - [`BgMsg::Progress`] (per-issue progress)
//! - [`BgMsg::GithubStatus`] (per-issue result)
//! - [`BgMsg::GithubStatusesDone`]

use std::path::PathBuf;

use tokio::sync::mpsc;

use crate::actions::Progress;
use crate::app::BgMsg;
use crate::github::{self, GithubStatus};

/// A single lookup to perform: `(issue_key, branch_prefix, repo_path)`.
pub type StatusLookup = (String, String, PathBuf);

/// Spawn GitHub status resolution for the given lookups.
///
/// No-ops if `lookups` is empty.
pub fn spawn(tx: mpsc::UnboundedSender<BgMsg>, lookups: Vec<StatusLookup>) {
    if lookups.is_empty() {
        return;
    }

    let total = lookups.len();
    tokio::spawn(async move {
        for (i, (key, branch_prefix, repo_path)) in lookups.into_iter().enumerate() {
            let _ = tx.send(BgMsg::Progress(Progress {
                action: "fetch_github_statuses",
                message: format!("Checking CI for {key}..."),
                current: i + 1,
                total,
            }));
            let result = github::find_pr_for_branch(&repo_path, &branch_prefix).await;
            let status = match result {
                Ok(Some(pr)) => GithubStatus::Found(pr),
                Ok(None) => GithubStatus::NoPr,
                Err(err) => GithubStatus::Error(err.to_string()),
            };
            let _ = tx.send(BgMsg::GithubStatus(key, status));
        }
        let _ = tx.send(BgMsg::GithubStatusesDone);
    });
}
