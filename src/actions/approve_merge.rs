//! **Approve & Auto-Merge** — approves a PR and enables auto-merge (squash).
//!
//! # Channel messages produced
//! - [`ActionMessage::TaskStarted`] / [`ActionMessage::TaskFinished`]
//! - [`ActionMessage::ApproveAutoMerged`]

use color_eyre::eyre::eyre;
use tokio::process::Command;
use tokio::sync::mpsc;

use super::ActionMessage;

pub fn spawn(tx: mpsc::UnboundedSender<ActionMessage>, repo_slug: String, pr_number: u64) {
    let tx = tx.clone();
    tokio::spawn(async move {
        let _ = tx.send(ActionMessage::TaskStarted(
            "Approving & merging".to_string(),
        ));
        let result = run(&repo_slug, pr_number).await;
        let _ = tx.send(ActionMessage::TaskFinished(
            "Approving & merging".to_string(),
        ));
        let _ = tx.send(ActionMessage::ApproveAutoMerged(result));
    });
}

async fn run(repo_slug: &str, pr_number: u64) -> color_eyre::Result<u64> {
    let pr_number_string = pr_number.to_string();

    let merge_output = Command::new("gh")
        .args([
            "pr",
            "merge",
            &pr_number_string,
            "--repo",
            repo_slug,
            "--auto",
            "--merge",
        ])
        .output()
        .await?;
    if !merge_output.status.success() {
        let stderr = String::from_utf8_lossy(&merge_output.stderr)
            .trim()
            .to_string();
        return Err(eyre!(
            "Failed to enable auto-merge on PR #{pr_number}: {}",
            if stderr.is_empty() {
                "unknown error"
            } else {
                &stderr
            }
        ));
    }

    let approve_output = Command::new("gh")
        .args([
            "pr",
            "review",
            &pr_number_string,
            "--repo",
            repo_slug,
            "--approve",
        ])
        .output()
        .await?;
    if !approve_output.status.success() {
        let stderr = String::from_utf8_lossy(&approve_output.stderr)
            .trim()
            .to_string();
        return Err(eyre!(
            "Failed to approve PR #{pr_number}: {}",
            if stderr.is_empty() {
                "unknown error"
            } else {
                &stderr
            }
        ));
    }

    Ok(pr_number)
}
