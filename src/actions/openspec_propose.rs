//! **Openspec Propose** — opens an opencode session that proposes an openspec change.
//!
//! 1. Build a slug from the issue key + summary
//! 2. Assemble context from ticket details, parent stories, and tagged repos
//! 3. Open a tmux window in REPOS_DIR with opencode as prompt
//!
//! # Channel messages produced
//! - [`Message::ActionStarted`] / [`Message::ActionFinished`]
//! - [`Message::OpenspecProposeOpened`]

use std::path::PathBuf;

use color_eyre::Result;
use tokio::process::Command;
use tokio::sync::mpsc;

use super::Message;
use crate::apis::jira::Issue;
use crate::git;

pub fn spawn(
    tx: mpsc::UnboundedSender<Message>,
    repos_dir: PathBuf,
    issue_key: String,
    issue_summary: String,
    issue_description: String,
    ancestors: Vec<Issue>,
    repo_slugs: Vec<String>,
) {
    super::spawn_action(
        tx,
        "openspec_propose",
        "Opening openspec propose",
        |tx| async move {
            let result: Result<String> = async {
                let slug = git::format_branch_name(&issue_key, &git::slugify(&issue_summary))
                    .to_ascii_lowercase();

                let mut context = format!(
                    "This change solves the following ticket: {issue_summary}\n{issue_description}"
                );
                context.push_str(&crate::issue::format_ancestor_context(&ancestors));
                if !repo_slugs.is_empty() {
                    context.push_str(&format!(
                        "\n\nTagged repositories: {}",
                        repo_slugs.join(", ")
                    ));
                }

                let prompt = format!("/opsx-propose {slug}\n\n{context}");
                let escaped_prompt = prompt.replace('\'', "'\\''");
                let shell_cmd = format!("opencode --prompt '{escaped_prompt}'");
                let dir = repos_dir.display().to_string();

                let _ = Command::new("tmux")
                    .args(["new-window", "-c", &dir])
                    .output()
                    .await;
                let _ = Command::new("tmux")
                    .args(["split-window", "-h", "-c", &dir, &shell_cmd])
                    .output()
                    .await;

                Ok(slug)
            }
            .await;
            let _ = tx.send(Message::OpenspecProposeOpened(result));
        },
    );
}
