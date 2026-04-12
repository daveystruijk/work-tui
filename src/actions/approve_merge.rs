//! **Approve & Auto-Merge** — approves a PR and enables auto-merge (squash).
//!
//! # Channel messages produced
//! - [`BgMsg::TaskStarted`] / [`BgMsg::TaskFinished`]
//! - [`BgMsg::ApproveAutoMerged`]

use color_eyre::eyre::eyre;
use tokio::process::Command;
use tokio::sync::mpsc;

use crate::app::BgMsg;

pub fn spawn(tx: mpsc::UnboundedSender<BgMsg>, repo_slug: String, pr_number: u64) {
    let tx = tx.clone();
    tokio::spawn(async move {
        let _ = tx.send(BgMsg::TaskStarted("Approving & merging"));
        let result = async {
            // Enable auto-merge
            let merge_output = Command::new("gh")
                .args([
                    "pr",
                    "merge",
                    &pr_number.to_string(),
                    "--repo",
                    &repo_slug,
                    "--auto",
                    "--merge",
                ])
                .output()
                .await?;
            if !merge_output.status.success() {
                let stderr = String::from_utf8_lossy(&merge_output.stderr).trim().to_string();
                return Err(eyre!(
                    "Failed to enable auto-merge on PR #{pr_number}: {}",
                    if stderr.is_empty() { "unknown error" } else { &stderr }
                ));
            }

            // Approve the PR
            let approve_output = Command::new("gh")
                .args([
                    "pr",
                    "review",
                    &pr_number.to_string(),
                    "--repo",
                    &repo_slug,
                    "--approve",
                ])
                .output()
                .await?;
            if !approve_output.status.success() {
                let stderr = String::from_utf8_lossy(&approve_output.stderr).trim().to_string();
                return Err(eyre!(
                    "Failed to approve PR #{pr_number}: {}",
                    if stderr.is_empty() { "unknown error" } else { &stderr }
                ));
            }

            Ok(pr_number)
        }
        .await;
        let _ = tx.send(BgMsg::TaskFinished("Approving & merging"));
        let _ = tx.send(BgMsg::ApproveAutoMerged(result));
    });
}
