//! **Pick Up** — claims an issue and creates a feature branch.
//!
//! Performs the full "pick up" workflow:
//! 1. Verify the repo working tree is clean
//! 2. Fetch from origin
//! 3. Create a new branch off `origin/main`
//! 4. Assign the issue to the current user
//! 5. Transition the issue to "In Progress" (if available)
//!
//! # Channel messages produced
//! - [`BgMsg::Progress`] (per-step progress)
//! - [`BgMsg::PickedUp`]

use std::path::PathBuf;

use color_eyre::eyre::eyre;
use tokio::sync::mpsc;

use crate::actions::Progress;
use crate::app::BgMsg;
use crate::git;
use crate::jira::JiraClient;

/// Spawn the pick-up workflow for a single issue.
pub fn spawn(
    tx: mpsc::UnboundedSender<BgMsg>,
    client: JiraClient,
    issue_key: String,
    issue_summary: String,
    repo_path: PathBuf,
    my_account_id: String,
) {
    tokio::spawn(async move {
        let result = async {
            // Step 1: Check clean state
            let _ = tx.send(BgMsg::Progress(Progress {
                action: "pick_up",
                message: "Checking working tree...".into(),
                current: 1,
                total: 5,
            }));
            if !git::is_clean(&repo_path).await? {
                return Err(eyre!("Repo has uncommitted changes"));
            }

            // Step 2: Fetch origin
            let _ = tx.send(BgMsg::Progress(Progress {
                action: "pick_up",
                message: "Fetching origin...".into(),
                current: 2,
                total: 5,
            }));
            git::fetch_origin(&repo_path).await?;

            // Step 3: Create branch
            let _ = tx.send(BgMsg::Progress(Progress {
                action: "pick_up",
                message: "Creating branch...".into(),
                current: 3,
                total: 5,
            }));
            let branch = git::create_branch_from_origin_main(
                &repo_path,
                &issue_key,
                &issue_summary,
            )
            .await?;

            // Step 4: Assign issue
            let _ = tx.send(BgMsg::Progress(Progress {
                action: "pick_up",
                message: "Assigning issue...".into(),
                current: 4,
                total: 5,
            }));
            client.assign_issue(&issue_key, &my_account_id).await?;

            // Step 5: Transition to In Progress
            let _ = tx.send(BgMsg::Progress(Progress {
                action: "pick_up",
                message: "Transitioning to In Progress...".into(),
                current: 5,
                total: 5,
            }));
            let transitions = client.get_transitions(&issue_key).await?;
            let progress = transitions
                .into_iter()
                .find(|t| t.name.to_lowercase().contains("progress"));
            if let Some(t) = progress {
                client.transition_issue(&issue_key, &t.id).await?;
            }

            Ok(branch)
        }
        .await;
        let _ = tx.send(BgMsg::PickedUp(result));
    });
}
