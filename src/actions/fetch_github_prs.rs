//! **Fetch GitHub PRs** — lists open PRs across all configured repositories.
//!
//! Uses a single `gh api graphql` call to fetch all configured repos and
//! collects the results into a [`BgMsg::GithubPrs`] delivery. Errors are
//! forwarded so the UI can surface them.
//!
//! # Channel messages produced
//! - [`BgMsg::Progress`] (per-repo progress)
//! - [`BgMsg::GithubPrs`]

use tokio::sync::mpsc;

use crate::app::BgMsg;
use crate::github;

/// Spawn GitHub PR fetching for all configured repos.
///
/// No-ops if `repos` is empty.
pub fn spawn(tx: mpsc::UnboundedSender<BgMsg>, repos: Vec<String>) {
    if repos.is_empty() {
        return;
    }

    let _ = tx.send(BgMsg::TaskStarted("Fetching PRs"));
    let tx = tx.clone();
    tokio::spawn(async move {
        let (all_prs, errors) = github::list_all_repo_prs(&repos).await;
        let _ = tx.send(BgMsg::TaskFinished("Fetching PRs"));
        let _ = tx.send(BgMsg::GithubPrs(all_prs, errors));
    });
}
