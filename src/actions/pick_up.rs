//! **Pick Up** — claims an issue and creates a feature branch.
//!
//! Performs the full "pick up" workflow:
//! 1. Note whether the working tree has local changes
//! 2. Fetch from origin
//! 3. Create a new branch off `origin/main`
//! 4. Assign the issue to the current user
//! 5. Transition the issue to "In Progress" (if available)
//! 6. Open a tmux pane with an opencode session for the repo
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
    repo_path: PathBuf,
    my_account_id: String,
    ancestors: Vec<Issue>,
) {
    super::spawn_action(tx, "pick_up", "Picking up", |tx| async move {
        let result: color_eyre::Result<PickUpResult> = async {
            let _ = tx.send(Message::Progress(Progress {
                task_id: "pick_up".into(),
                message: "Inspecting working tree...".into(),
                current: 1,
                total: 6,
            }));
            let has_uncommitted_changes = !git::is_clean(&repo_path).await?;

            let _ = tx.send(Message::Progress(Progress {
                task_id: "pick_up".into(),
                message: "Fetching origin...".into(),
                current: 2,
                total: 6,
            }));
            git::fetch_origin(&repo_path).await?;

            let _ = tx.send(Message::Progress(Progress {
                task_id: "pick_up".into(),
                message: "Creating or reusing branch...".into(),
                current: 3,
                total: 6,
            }));
            let branch_setup =
                git::create_branch_from_origin_main(&repo_path, &issue_key, &issue_summary).await?;

            if branch_setup.reused_existing {
                let _ = tx.send(Message::Progress(Progress {
                    task_id: "pick_up".into(),
                    message: "Opening opencode session...".into(),
                    current: 4,
                    total: 4,
                }));
                let mut context = format!("{issue_summary}\n\n{issue_description}");
                context.push_str(&crate::issue::format_ancestor_context(&ancestors));
                let escaped_prompt = context.replace('\'', "'\\''");
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
            } else {
                let _ = tx.send(Message::Progress(Progress {
                    task_id: "pick_up".into(),
                    message: "Assigning issue...".into(),
                    current: 4,
                    total: 7,
                }));
                client.assign_issue(&issue_key, &my_account_id).await?;

                let _ = tx.send(Message::Progress(Progress {
                    task_id: "pick_up".into(),
                    message: "Transitioning to In Progress and moving issue to board...".into(),
                    current: 5,
                    total: 7,
                }));
                let transitions = client.get_transitions(&issue_key).await?;
                let in_progress_transition = transitions
                    .iter()
                    .find(|t| t.name.to_lowercase().contains("progress"))
                    .or_else(|| {
                        transitions
                            .iter()
                            .find(|t| t.name.to_lowercase().contains("start"))
                    })
                    .ok_or_else(|| {
                        let names = transitions
                            .iter()
                            .map(|t| t.name.clone())
                            .collect::<Vec<_>>()
                            .join(", ");
                        eyre!("No 'In Progress' transition found. Available: {names}")
                    })?;
                client
                    .transition_issue(&issue_key, &in_progress_transition.id)
                    .await?;
                client.move_issue_to_active_board(&issue_key).await?;

                if !has_uncommitted_changes {
                    let _ = tx.send(Message::Progress(Progress {
                        task_id: "pick_up".into(),
                        message: "Opening opencode session...".into(),
                        current: 6,
                        total: 7,
                    }));
                    let mut context = format!("{issue_summary}\n\n{issue_description}");
                    context.push_str(&crate::issue::format_ancestor_context(&ancestors));
                    let escaped_prompt = context.replace('\'', "'\\''");
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
                } else {
                    let _ = tx.send(Message::Progress(Progress {
                        task_id: "pick_up".into(),
                        message: "Skipping opencode (uncommitted changes)...".into(),
                        current: 6,
                        total: 7,
                    }));
                }
            }

            Ok(PickUpResult {
                branch: branch_setup.branch_name,
            })
        }
        .await;
        let _ = tx.send(Message::PickedUp(result));
    });
}
