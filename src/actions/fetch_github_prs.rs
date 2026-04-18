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

    tokio::spawn(async move {
        let _ = tx.send(ActionMessage::TaskStarted("Fetching PRs".to_string()));
        let (all_prs, errors) = run(&repos).await.expect("fetch PRs");
        let _ = tx.send(ActionMessage::TaskFinished("Fetching PRs".to_string()));
        let _ = tx.send(ActionMessage::GithubPrs(all_prs, errors));
    });
}

async fn run(repos: &[String]) -> color_eyre::Result<(Vec<github::PrInfo>, Vec<String>)> {
    Ok(github::list_all_repo_prs(repos).await)
}
