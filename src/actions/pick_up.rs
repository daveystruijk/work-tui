//! **Pick Up** — claims an issue and optionally creates a feature branch.
//!
//! Performs the full "pick up" workflow:
//! 1. If a repo is linked, require a clean working tree unless carrying changes, then checkout/create branch
//! 2. Open a tmux pane with an opencode session
//! 3. Assign the issue to the current user
//! 4. Transition the issue to "In Progress" (if available)
//!
//! Jira mutations (assign, transition, board move) only happen after all git
//! operations succeed. A dirty working tree aborts the entire action unless the
//! user chose to carry uncommitted changes onto the target branch.
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
#[allow(clippy::too_many_arguments)]
pub fn spawn(
    tx: mpsc::UnboundedSender<Message>,
    client: JiraClient,
    issue_key: String,
    issue_summary: String,
    issue_description: String,
    repo_path: Option<PathBuf>,
    base_ref: String,
    carry_changes: bool,
    my_account_id: String,
    ancestors: Vec<Issue>,
) {
    super::spawn_action(tx, "pick_up", "Picking up", move |tx| async move {
        let result: color_eyre::Result<PickUpResult> = async {
            let has_linked_repo = repo_path.is_some();
            let total_steps = if has_linked_repo { 6 } else { 3 };
            let branch_name = if let Some(repo_path) = repo_path.as_ref() {
                let _ = tx.send(Message::Progress(Progress {
                    task_id: "pick_up".into(),
                    message: "Inspecting working tree...".into(),
                    current: 1,
                    total: total_steps,
                }));
                if !carry_changes && !git::is_clean(repo_path).await? {
                    return Err(eyre!(
                        "Working tree has uncommitted changes — commit or stash first"
                    ));
                }

                let _ = tx.send(Message::Progress(Progress {
                    task_id: "pick_up".into(),
                    message: "Fetching origin...".into(),
                    current: 2,
                    total: total_steps,
                }));
                git::fetch_origin(repo_path).await?;

                let _ = tx.send(Message::Progress(Progress {
                    task_id: "pick_up".into(),
                    message: format!("Creating or reusing branch (off {base_ref})..."),
                    current: 3,
                    total: total_steps,
                }));
                let branch_setup = git::create_branch_from(
                    repo_path,
                    &issue_key,
                    &issue_summary,
                    &base_ref,
                    carry_changes,
                )
                .await?;

                Some(branch_setup.branch_name)
            } else {
                None
            };

            let _ = tx.send(Message::Progress(Progress {
                task_id: "pick_up".into(),
                message: "Opening opencode session...".into(),
                current: if has_linked_repo { 4 } else { 1 },
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
            let repo_dir = repo_path.as_ref().cloned().map(Ok).unwrap_or_else(|| {
                dirs::home_dir()
                    .map(|dir| dir.join("momo"))
                    .ok_or_else(|| eyre!("HOME is not set; cannot resolve ~/momo"))
            })?;
            let repo_dir = repo_dir.display().to_string();

            tokio::spawn(async move {
                let _ = Command::new("tmux")
                    .args(["new-window", "-c", &repo_dir])
                    .output()
                    .await;
                let _ = Command::new("tmux")
                    .args(["split-window", "-h", "-c", &repo_dir, &shell_cmd])
                    .output()
                    .await;
            });

            let _ = tx.send(Message::Progress(Progress {
                task_id: "pick_up".into(),
                message: "Assigning issue...".into(),
                current: if has_linked_repo { 5 } else { 2 },
                total: total_steps,
            }));
            client.assign_issue(&issue_key, &my_account_id).await?;

            let _ = tx.send(Message::Progress(Progress {
                task_id: "pick_up".into(),
                message: "Transitioning to In Progress and moving issue to board...".into(),
                current: if has_linked_repo { 6 } else { 3 },
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

            Ok(PickUpResult {
                branch: branch_name,
            })
        }
        .await;
        let _ = tx.send(Message::PickedUp(result));
    });
}
