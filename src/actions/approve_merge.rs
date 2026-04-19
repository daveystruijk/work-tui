//! **Approve & Auto-Merge** — approves a PR and enables auto-merge (squash).
//!
//! # Channel messages produced
//! - [`Message::ActionStarted`] / [`Message::ActionFinished`]
//! - [`Message::ApproveAutoMerged`]

use color_eyre::eyre::eyre;
use tokio::process::Command;
use tokio::sync::mpsc;

use super::Message;

pub fn spawn(tx: mpsc::UnboundedSender<Message>, repo_slug: String, pr_number: u64) {
    super::spawn_action(
        tx,
        "approve_merge",
        "Approving & merging",
        move |tx| async move {
            let result: color_eyre::Result<u64> = async {
                let pr_number_string = pr_number.to_string();

                let merge_output = Command::new("gh")
                    .args([
                        "pr",
                        "merge",
                        &pr_number_string,
                        "--repo",
                        &repo_slug,
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
                        &repo_slug,
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
            .await;
            let _ = tx.send(Message::ApproveAutoMerged(result));
        },
    );
}
