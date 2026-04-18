//! **Initialize** — bootstraps the application on startup.
//!
//! Spawns three independent tasks in parallel:
//! 1. Resolve the current git branch
//! 2. Fetch the Jira user identity
//! 3. Fetch the initial issue list
//!
//! # Channel messages produced
//! - [`ActionMessage::CurrentBranch`]
//! - [`ActionMessage::Myself`]
//! - [`ActionMessage::Issues`]
//! - [`ActionMessage::Progress`] (one per sub-task)

use tokio::sync::mpsc;

use super::ActionMessage;
use crate::actions::Progress;
use crate::apis::jira::JiraClient;
use crate::git;

/// Spawn all initialization tasks concurrently.
pub fn spawn(tx: mpsc::UnboundedSender<ActionMessage>, client: JiraClient, jql: String) {
    tokio::spawn(async move {
        let _ = tx.send(ActionMessage::TaskStarted("Initializing".to_string()));
        run(&tx, &client, &jql).await;
        let _ = tx.send(ActionMessage::TaskFinished("Initializing".to_string()));
    });
}

async fn run(tx: &mpsc::UnboundedSender<ActionMessage>, client: &JiraClient, jql: &str) {
    let current_branch = async {
        let _ = tx.send(ActionMessage::Progress(Progress {
            action: "initialize",
            message: "Resolving git branch...".into(),
            current: 1,
            total: 3,
        }));
        let branch = git::current_branch()
            .await
            .unwrap_or_else(|_| "(detached)".to_string());
        let _ = tx.send(ActionMessage::CurrentBranch(branch));
    };

    let myself = async {
        let _ = tx.send(ActionMessage::Progress(Progress {
            action: "initialize",
            message: "Fetching Jira identity...".into(),
            current: 2,
            total: 3,
        }));
        let result = client
            .get_myself()
            .await
            .map(|u| u.account_id.unwrap_or_default());
        let _ = tx.send(ActionMessage::Myself(result));
    };

    let issues = async {
        let _ = tx.send(ActionMessage::Progress(Progress {
            action: "initialize",
            message: "Fetching issues...".into(),
            current: 3,
            total: 3,
        }));
        let result = client.search(jql).await;
        let _ = tx.send(ActionMessage::Issues(result));
    };

    tokio::join!(current_branch, myself, issues);
}
