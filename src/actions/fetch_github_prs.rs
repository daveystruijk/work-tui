//! **Fetch GitHub PRs** — searches for open PRs by branch prefix.
//!
//! Uses a single GitHub search query with `head:<project_key>-` to find all
//! open PRs whose branch starts with the project key (e.g. `INI-`).
//! This returns only relevant PRs in one fast request.
//!
//! # Channel messages produced
//! - [`Message::GithubPrs`]

use tokio::sync::mpsc;

use super::Message;
use crate::apis::github;

/// Spawn GitHub PR search for branches starting with `head_prefix` in `org`.
pub fn spawn(tx: mpsc::UnboundedSender<Message>, org: String, head_prefix: String) {
    if org.is_empty() || head_prefix.is_empty() {
        return;
    }

    super::spawn_action(tx, "fetch_github_prs", "Fetching PRs", |tx| async move {
        let (all_prs, errors) = github::search_org_prs(&org, &head_prefix).await;
        let _ = tx.send(Message::GithubPrs(all_prs, errors));
    });
}
