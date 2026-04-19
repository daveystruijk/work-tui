//! **Fetch GitHub PRs** — lists open PRs across all configured repositories.
//!
//! Uses a single `gh api graphql` call to fetch all configured repos and
//! collects the results into a [`Message::GithubPrs`] delivery. Errors are
//! forwarded so the UI can surface them.
//!
//! # Channel messages produced
//! - [`Message::Progress`] (per-repo progress)
//! - [`Message::GithubPrs`]

use tokio::sync::mpsc;

use super::Message;
use crate::apis::github;

/// Spawn GitHub PR fetching for all configured repos.
///
/// No-ops if `repos` is empty.
pub fn spawn(tx: mpsc::UnboundedSender<Message>, repos: Vec<String>) {
    if repos.is_empty() {
        return;
    }

    super::spawn_action(tx, "fetch_github_prs", "Fetching PRs", |tx| async move {
        let (all_prs, errors) = github::list_all_repo_prs(&repos).await;
        let _ = tx.send(Message::GithubPrs(all_prs, errors));
    });
}
