//! **Fetch GitHub PRs** — searches for open PRs matching issue keys.
//!
//! Uses a single `gh api graphql` call with aliased `search()` queries —
//! one per issue key — to find PRs whose branch name starts with the key.
//! This is much cheaper than fetching all open PRs per repository.
//!
//! # Channel messages produced
//! - [`Message::GithubPrs`]

use tokio::sync::mpsc;

use super::Message;
use crate::apis::github;

/// Spawn GitHub PR search for the given issue keys within an org.
///
/// No-ops if `issue_keys` is empty.
pub fn spawn(tx: mpsc::UnboundedSender<Message>, org: String, issue_keys: Vec<String>) {
    if issue_keys.is_empty() {
        return;
    }

    super::spawn_action(tx, "fetch_github_prs", "Fetching PRs", |tx| async move {
        let (all_prs, errors) = github::search_prs_by_issue_keys(&org, &issue_keys).await;
        let _ = tx.send(Message::GithubPrs(all_prs, errors));
    });
}
