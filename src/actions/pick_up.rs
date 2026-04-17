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
//! - [`ActionMessage::Progress`] (per-step progress)
//! - [`ActionMessage::PickedUp`]

use std::path::PathBuf;

use tokio::process::Command;
use tokio::sync::mpsc;

use super::{ActionMessage, PickUpResult};
use crate::actions::Progress;
use crate::git;
use crate::apis::jira::JiraClient;

/// Spawn the pick-up workflow for a single issue.
pub fn spawn(
    tx: mpsc::UnboundedSender<ActionMessage>,
    client: JiraClient,
    issue_key: String,
    issue_summary: String,
    issue_description: String,
    repo_path: PathBuf,
    my_account_id: String,
) {
    let tx = tx.clone();
    tokio::spawn(async move {
        let _ = tx.send(ActionMessage::TaskStarted("Picking up"));
        let result = async {
            // Step 1: Check working tree state
            let _ = tx.send(ActionMessage::Progress(Progress {
                action: "pick_up",
                message: "Inspecting working tree...".into(),
                current: 1,
                total: 6,
            }));
            let has_uncommitted_changes = !git::is_clean(&repo_path).await?;

            // Step 2: Fetch origin
            let _ = tx.send(ActionMessage::Progress(Progress {
                action: "pick_up",
                message: "Fetching origin...".into(),
                current: 2,
                total: 6,
            }));
            git::fetch_origin(&repo_path).await?;

            // Step 3: Create or reuse branch
            let _ = tx.send(ActionMessage::Progress(Progress {
                action: "pick_up",
                message: "Creating or reusing branch...".into(),
                current: 3,
                total: 6,
            }));
            let branch_setup =
                git::create_branch_from_origin_main(&repo_path, &issue_key, &issue_summary).await?;

            // Step 4: Assign issue
            let _ = tx.send(ActionMessage::Progress(Progress {
                action: "pick_up",
                message: "Assigning issue...".into(),
                current: 4,
                total: 6,
            }));
            client.assign_issue(&issue_key, &my_account_id).await?;

            // Step 5: Transition to In Progress and ensure board visibility
            let _ = tx.send(ActionMessage::Progress(Progress {
                action: "pick_up",
                message: "Transitioning to In Progress and moving issue to board...".into(),
                current: 5,
                total: 6,
            }));
            let transitions = client.get_transitions(&issue_key).await?;
            let progress = transitions
                .into_iter()
                .find(|t| t.name.to_lowercase().contains("progress"));
            if let Some(t) = progress {
                client.transition_issue(&issue_key, &t.id).await?;
            }
            client.move_issue_to_active_board(&issue_key).await?;

            let should_open_opencode = branch_setup.reused_existing || !has_uncommitted_changes;
            if should_open_opencode {
                let _ = tx.send(ActionMessage::Progress(Progress {
                    action: "pick_up",
                    message: "Opening opencode session...".into(),
                    current: 6,
                    total: 6,
                }));
                let prompt = format!("{issue_summary}\n\n{issue_description}");
                let escaped_prompt = prompt.replace('\'', "'\\''");
                let shell_cmd = format!("opencode --prompt '{escaped_prompt}'");
                let repo_dir = repo_path.display().to_string();

                // Create a new tmux tab (window) in the repo directory, then
                // split it and run opencode in the new pane.
                let _ = Command::new("tmux")
                    .args(["new-window", "-c", &repo_dir])
                    .output()
                    .await;
                let _ = Command::new("tmux")
                    .args(["split-window", "-h", "-c", &repo_dir, &shell_cmd])
                    .output()
                    .await;
            } else {
                let _ = tx.send(ActionMessage::Progress(Progress {
                    action: "pick_up",
                    message: "Skipping opencode (uncommitted changes)...".into(),
                    current: 6,
                    total: 6,
                }));
            }

            Ok(PickUpResult {
                branch: branch_setup.branch_name,
                skipped_opencode: has_uncommitted_changes && !branch_setup.reused_existing,
            })
        }
        .await;
        let _ = tx.send(ActionMessage::TaskFinished("Picking up"));
        let _ = tx.send(ActionMessage::PickedUp(result));
    });
}
