//! **Poll CI status** — periodically re-fetches PRs and detects CI status changes.
//!
//! Runs every N seconds in the background, compares check statuses against
//! the previous snapshot, and emits [`BgMsg::CiStatusChanged`] when any PR's
//! CI status transitions.

use std::collections::HashMap;
use std::time::Duration;

use tokio::sync::mpsc;

use crate::app::BgMsg;
use crate::github::{self, CheckStatus, PrInfo};

/// A single CI status transition detected during polling.
#[derive(Debug, Clone)]
pub struct CiChange {
    pub pr_number: u64,
    pub head_branch: String,
    pub repo_slug: String,
    pub old_status: CheckStatus,
    pub new_status: CheckStatus,
}

/// Spawn the CI polling loop.
///
/// No-ops if `repos` is empty.
pub fn spawn(tx: mpsc::UnboundedSender<BgMsg>, repos: Vec<String>, interval: Duration) {
    if repos.is_empty() {
        return;
    }

    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        let mut prev_statuses: HashMap<u64, CheckStatus> = HashMap::new();
        let mut first_run = true;

        loop {
            ticker.tick().await;

            let futures: Vec<_> = repos.iter().map(|r| github::list_repo_prs(r)).collect();
            let results = futures::future::join_all(futures).await;
            let all_prs: Vec<PrInfo> = results
                .into_iter()
                .filter_map(|r| r.ok())
                .flatten()
                .collect();

            if !first_run {
                let changes: Vec<CiChange> = all_prs
                    .iter()
                    .filter_map(|pr| {
                        let old = prev_statuses.get(&pr.number)?;
                        if *old == pr.checks {
                            return None;
                        }
                        Some(CiChange {
                            pr_number: pr.number,
                            head_branch: pr.head_branch.clone(),
                            repo_slug: pr.repo_slug.clone(),
                            old_status: old.clone(),
                            new_status: pr.checks.clone(),
                        })
                    })
                    .collect();

                if !changes.is_empty() {
                    let _ = tx.send(BgMsg::CiStatusChanged(changes));
                }
            }

            prev_statuses = all_prs
                .iter()
                .map(|pr| (pr.number, pr.checks.clone()))
                .collect();
            first_run = false;

            let _ = tx.send(BgMsg::GithubPrs(all_prs));
        }
    });
}
