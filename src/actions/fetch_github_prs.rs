//! **Fetch GitHub PRs** — lists open PRs across all configured repositories.
//!
//! Uses a single `gh api graphql` call to fetch all configured repos and
//! collects the results into a [`ActionMessage::GithubPrs`] delivery. Errors are
//! forwarded so the UI can surface them.
//!
//! # Channel messages produced
//! - [`ActionMessage::Progress`] (per-repo progress)
//! - [`ActionMessage::GithubPrs`]

use tokio::sync::mpsc;

use super::ActionMessage;
use crate::apis::github;

/// Spawn GitHub PR fetching for all configured repos.
///
/// No-ops if `repos` is empty.
pub fn spawn(tx: mpsc::UnboundedSender<ActionMessage>, repos: Vec<String>) {
    if repos.is_empty() {
        return;
    }

    let _ = tx.send(ActionMessage::TaskStarted("Fetching PRs"));
    let tx = tx.clone();
    tokio::spawn(async move {
        let (all_prs, errors) = github::list_all_repo_prs(&repos).await;
        let _ = tx.send(ActionMessage::TaskFinished("Fetching PRs"));
        let _ = tx.send(ActionMessage::GithubPrs(all_prs, errors));
    });
}
