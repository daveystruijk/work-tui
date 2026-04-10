//! **Fetch GitHub PRs** — lists open PRs across all configured repositories.
//!
//! Runs one `gh pr list` per repo in parallel and collects the results into a
//! single [`BgMsg::GithubPrs`] delivery.
//!
//! # Channel messages produced
//! - [`BgMsg::Progress`] (per-repo progress)
//! - [`BgMsg::GithubPrs`]

use tokio::sync::mpsc;

use crate::app::BgMsg;
use crate::github::{self, PrInfo};

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
        let futures: Vec<_> = repos.iter().map(|r| github::list_repo_prs(r)).collect();
        let results = futures::future::join_all(futures).await;
        let all_prs: Vec<PrInfo> = results
            .into_iter()
            .filter_map(|r| r.ok())
            .flatten()
            .collect();
        let _ = tx.send(BgMsg::TaskFinished("Fetching PRs"));
        let _ = tx.send(BgMsg::GithubPrs(all_prs));
    });
}
