//! **Pick Up** — claims an issue and optionally creates a feature branch.
//!
//! Performs the full "pick up" workflow:
//! 1. If a repo is linked, inspect the working tree and create/reuse a branch
//! 2. Assign the issue to the current user
//! 3. Transition the issue to "In Progress" (if available)
//! 4. Open a tmux pane with an opencode session
//!
//! # Channel messages produced
//! - [`Message::Progress`] (per-step progress)
//! - [`Message::PickedUp`]

use std::path::PathBuf;

use color_eyre::eyre::eyre;
use tokio::process::Command;
use tokio::sync::mpsc;

use super::{Message, PickUpResult};
use crate::actions::Progress;
use crate::apis::jira::{Issue, JiraClient};
use crate::git;

/// Spawn the pick-up workflow for a single issue.
pub fn spawn(
    tx: mpsc::UnboundedSender<Message>,
    client: JiraClient,
    issue_key: String,
    issue_summary: String,
    issue_description: String,
    repo_path: Option<PathBuf>,
    my_account_id: String,
    ancestors: Vec<Issue>,
) {
    super::spawn_action(tx, "pick_up", "Picking up", |tx| async move {
        let result: color_eyre::Result<PickUpResult> = async {
            let has_linked_repo = repo_path.is_some();
            let total_steps = if has_linked_repo { 6 } else { 3 };
            let branch_setup = if let Some(repo_path) = repo_path.as_ref() {
                let _ = tx.send(Message::Progress(Progress {
                    task_id: "pick_up".into(),
                    message: "Inspecting working tree...".into(),
                    current: 1,
                    total: total_steps,
                }));
                let has_uncommitted_changes = !git::is_clean(repo_path).await?;

                let _ = tx.send(Message::Progress(Progress {
                    task_id: "pick_up".into(),
                    message: "Fetching origin...".into(),
                    current: 2,
                    total: total_steps,
                }));
                git::fetch_origin(repo_path).await?;

                let _ = tx.send(Message::Progress(Progress {
                    task_id: "pick_up".into(),
                    message: "Creating or reusing branch...".into(),
                    current: 3,
                    total: total_steps,
                }));
                let branch_setup =
                    git::create_branch_from_origin_main(repo_path, &issue_key, &issue_summary)
                        .await?;

                Some((branch_setup, has_uncommitted_changes))
            } else {
                None
            };

            let _ = tx.send(Message::Progress(Progress {
                task_id: "pick_up".into(),
                message: "Assigning issue...".into(),
                current: if has_linked_repo { 4 } else { 1 },
                total: total_steps,
            }));
            client.assign_issue(&issue_key, &my_account_id).await?;

            let _ = tx.send(Message::Progress(Progress {
                task_id: "pick_up".into(),
                message: "Transitioning to In Progress and moving issue to board...".into(),
                current: if has_linked_repo { 5 } else { 2 },
                total: total_steps,
            }));
            let transitions = client.get_transitions(&issue_key).await?;
            if let Some(in_progress_transition) = transitions
                .iter()
                .find(|t| t.name.to_lowercase().contains("progress"))
                .or_else(|| {
                    transitions
                        .iter()
                        .find(|t| t.name.to_lowercase().contains("start"))
                })
            {
                client
                    .transition_issue(&issue_key, &in_progress_transition.id)
                    .await?;
            }
            client.move_issue_to_active_board(&issue_key).await?;

            if branch_setup
                .as_ref()
                .map(|(_, dirty)| !dirty)
                .unwrap_or(true)
            {
                let _ = tx.send(Message::Progress(Progress {
                    task_id: "pick_up".into(),
                    message: "Opening opencode session...".into(),
                    current: if has_linked_repo { 6 } else { 3 },
                    total: total_steps,
                }));
                let prompt = crate::issue::format_pick_up_prompt(
                    &issue_key,
                    &issue_summary,
                    &issue_description,
                    &ancestors,
                );
                let escaped_prompt = prompt.replace('\'', "'\\''");
                let shell_cmd = format!("opencode --prompt '{escaped_prompt}'");
                let repo_dir = branch_setup
                    .as_ref()
                    .and_then(|_| repo_path.as_ref())
                    .cloned()
                    .map(Ok)
                    .unwrap_or_else(|| {
                        dirs::home_dir()
                            .map(|dir| dir.join("momo"))
                            .ok_or_else(|| eyre!("HOME is not set; cannot resolve ~/momo"))
                    })?;
                let repo_dir = repo_dir.display().to_string();

                let _ = Command::new("tmux")
                    .args(["new-window", "-c", &repo_dir])
                    .output()
                    .await;
                let _ = Command::new("tmux")
                    .args(["split-window", "-h", "-c", &repo_dir, &shell_cmd])
                    .output()
                    .await;
            } else {
                let _ = tx.send(Message::Progress(Progress {
                    task_id: "pick_up".into(),
                    message: "Skipping opencode (uncommitted changes)...".into(),
                    current: 6,
                    total: total_steps,
                }));
            }

            Ok(PickUpResult {
                branch: branch_setup.map(|(setup, _)| setup.branch_name),
            })
        }
        .await;
        let _ = tx.send(Message::PickedUp(result));
    });
}
