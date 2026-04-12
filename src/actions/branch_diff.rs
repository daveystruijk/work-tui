//! **Branch Diff** — checks out an issue branch and opens a difftool against main.
//!
//! # Channel messages produced
//! - [`BgMsg::TaskStarted`] / [`BgMsg::TaskFinished`]
//! - [`BgMsg::BranchDiffOpened`]

use std::path::PathBuf;

use color_eyre::eyre::eyre;
use tokio::process::Command;
use tokio::sync::mpsc;

use crate::app::BgMsg;
use crate::git;

pub fn spawn(tx: mpsc::UnboundedSender<BgMsg>, issue_key: String, repo_path: PathBuf) {
    let tx = tx.clone();
    tokio::spawn(async move {
        let _ = tx.send(BgMsg::TaskStarted("Opening diff"));
        let result = async {
            // Fetch first so remote-only branches are visible
            git::fetch_origin(&repo_path).await?;

            // Search local and remote branches for one matching the issue key
            let branch_output = Command::new("git")
                .args(["branch", "--all", "--list"])
                .current_dir(&repo_path)
                .output()
                .await?;
            if !branch_output.status.success() {
                let stderr = String::from_utf8_lossy(&branch_output.stderr);
                return Err(eyre!("Failed to list branches: {stderr}"));
            }
            let stdout = String::from_utf8(branch_output.stdout)
                .map_err(|err| eyre!("Invalid UTF-8 from git branch: {err}"))?;
            let issue_key_lower = issue_key.to_lowercase();

            // Prefer a local branch; fall back to remote tracking branch
            let clean_name = |line: &str| -> String {
                let trimmed = line.trim();
                let trimmed = trimmed.strip_prefix("* ").unwrap_or(trimmed);
                // Strip "remotes/origin/" prefix for remote branches
                trimmed
                    .strip_prefix("remotes/origin/")
                    .unwrap_or(trimmed)
                    .to_string()
            };

            let branch = stdout
                .lines()
                .filter(|line| !line.contains("->")) // skip HEAD -> origin/main aliases
                .map(|line| clean_name(line))
                .filter(|name| name.to_lowercase().starts_with(&issue_key_lower))
                .next()
                .ok_or_else(|| eyre!("No branch found for {issue_key}"))?;

            let checkout = Command::new("git")
                .args(["checkout", &branch])
                .current_dir(&repo_path)
                .output()
                .await?;
            if !checkout.status.success() {
                let stderr = String::from_utf8_lossy(&checkout.stderr);
                return Err(eyre!("Failed to checkout {branch}: {stderr}"));
            }

            let repo_dir = repo_path.display().to_string();
            let new_window = Command::new("tmux")
                .args(["new-window", "-c", &repo_dir])
                .output()
                .await?;
            if !new_window.status.success() {
                let stderr = String::from_utf8_lossy(&new_window.stderr);
                return Err(eyre!("Failed to create tmux window: {stderr}"));
            }

            let difftool_cmd = "git diff origin/main...HEAD";
            let send_keys = Command::new("tmux")
                .args(["send-keys", difftool_cmd, "Enter"])
                .output()
                .await?;
            if !send_keys.status.success() {
                let stderr = String::from_utf8_lossy(&send_keys.stderr);
                return Err(eyre!("Failed to start difftool: {stderr}"));
            }

            Ok(branch)
        }
        .await;
        let _ = tx.send(BgMsg::TaskFinished("Opening diff"));
        let _ = tx.send(BgMsg::BranchDiffOpened(result));
    });
}
