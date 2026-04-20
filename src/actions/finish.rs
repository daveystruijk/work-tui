//! **Finish** — pushes changes, creates a PR, and moves the ticket to Review.
//!
//! Performs the full "finish" workflow:
//! 1. Verify the repo working tree is clean
//! 2. Fetch from origin
//! 3. Push the branch to origin
//! 4. Create a pull request via `gh pr create`
//! 5. Transition the Jira issue to "Review" (if available)
//!
//! # Channel messages produced
//! - [`Message::Progress`] (per-step progress)
//! - [`Message::Finished`]

use std::path::PathBuf;

use color_eyre::eyre::eyre;
use tokio::sync::mpsc;

use super::Message;
use crate::actions::Progress;
use crate::apis::github;
use crate::apis::jira::JiraClient;
use crate::git;

/// Spawn the finish workflow for a single issue.
pub fn spawn(
    tx: mpsc::UnboundedSender<Message>,
    client: JiraClient,
    issue_key: String,
    issue_summary: String,
    repo_path: PathBuf,
) {
    super::spawn_action(tx, "finish", "Finishing", |tx| async move {
        let result: color_eyre::Result<String> = async {
            let branch = git::current_branch_in(&repo_path).await?;
            if branch.is_empty() {
                return Err(eyre!("Cannot finish: no branch checked out"));
            }
            if branch == "main" || branch == "master" {
                return Err(eyre!("Cannot finish: on {branch}, not a feature branch"));
            }

            let _ = tx.send(Message::Progress(Progress {
                task_id: "finish".into(),
                message: "Checking working tree...".into(),
                current: 1,
                total: 5,
            }));
            if !git::is_clean(&repo_path).await? {
                let commit_message = format!("{issue_key} {issue_summary}");
                git::commit_all(&repo_path, &commit_message).await?;
            }

            let _ = tx.send(Message::Progress(Progress {
                task_id: "finish".into(),
                message: "Fetching origin...".into(),
                current: 2,
                total: 5,
            }));
            git::fetch_origin(&repo_path).await?;

            let _ = tx.send(Message::Progress(Progress {
                task_id: "finish".into(),
                message: "Pushing branch...".into(),
                current: 3,
                total: 5,
            }));
            let pr_title = format!("{issue_key} {issue_summary}");
            git::push_branch(&repo_path, &branch).await?;

            let _ = tx.send(Message::Progress(Progress {
                task_id: "finish".into(),
                message: "Creating pull request...".into(),
                current: 4,
                total: 5,
            }));
            let pr_url = github::create_pr(&repo_path, &pr_title, "").await?;

            let _ = tx.send(Message::Progress(Progress {
                task_id: "finish".into(),
                message: "Transitioning to Review...".into(),
                current: 5,
                total: 5,
            }));
            let transitions = client.get_transitions(&issue_key).await?;
            let review_transition = transitions
                .iter()
                .find(|t| t.name.to_lowercase().contains("review"))
                .or_else(|| {
                    transitions
                        .iter()
                        .find(|t| t.name.to_lowercase().contains("done"))
                })
                .ok_or_else(|| {
                    let names = transitions
                        .iter()
                        .map(|t| t.name.clone())
                        .collect::<Vec<_>>()
                        .join(", ");
                    eyre!("No 'Review' transition found. Available: {names}")
                })?;
            client
                .transition_issue(&issue_key, &review_transition.id)
                .await?;

            Ok(pr_url)
        }
        .await;
        let _ = tx.send(Message::Finished(result));
    });
}
