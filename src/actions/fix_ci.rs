//! **Fix CI** — checks out the issue branch and opens opencode with the CI error.
//!
//! 1. Fetch origin
//! 2. Checkout the PR's head branch
//! 3. Open a tmux window + split pane running opencode with the CI error as prompt
//!
//! # Channel messages produced
//! - [`Message::ActionStarted`] / [`Message::ActionFinished`]
//! - [`Message::FixCiOpened`]

use std::path::PathBuf;

use color_eyre::Result;
use tokio::process::Command;
use tokio::sync::mpsc;

use super::Message;
use crate::git;

pub fn spawn(
    tx: mpsc::UnboundedSender<Message>,
    repo_path: PathBuf,
    branch: String,
    ci_error: String,
) {
    super::spawn_action(tx, "fix_ci", "Fixing CI", |tx| async move {
        let result: Result<String> = async {
            git::fetch_origin(&repo_path).await?;
            git::checkout_branch(&repo_path, &branch).await?;

            let prompt = format!(
                "The CI pipeline failed. Here is the error output:\n\n{ci_error}\n\nPlease fix the issue."
            );
            let escaped_prompt = prompt.replace('\'', "'\\''");
            let shell_cmd = format!("opencode --prompt '{escaped_prompt}'");
            let repo_dir = repo_path.display().to_string();

            let _ = Command::new("tmux")
                .args(["new-window", "-c", &repo_dir])
                .output()
                .await;
            let _ = Command::new("tmux")
                .args(["split-window", "-h", "-c", &repo_dir, &shell_cmd])
                .output()
                .await;

            Ok(branch.to_string())
        }
        .await;
        let _ = tx.send(Message::FixCiOpened(result));
    });
}
