//! **Initialize** — bootstraps the application on startup.
//!
//! Spawns three independent tasks in parallel:
//! 1. Resolve the current git branch
//! 2. Fetch the Jira user identity
//! 3. Fetch the initial issue list
//!
//! # Channel messages produced
//! - [`Message::CurrentBranch`]
//! - [`Message::Myself`]
//! - [`Message::Issues`]
//! - [`Message::Progress`] (one per sub-task)

use tokio::sync::mpsc;

use super::Message;
use crate::actions::Progress;
use crate::apis::jira::JiraClient;
use crate::git;

/// Spawn all initialization tasks concurrently.
pub fn spawn(tx: mpsc::UnboundedSender<Message>, client: JiraClient, jql: String) {
    super::spawn_action(tx, "initialize", "Initializing", |tx| async move {
        let current_branch = async {
            let _ = tx.send(Message::Progress(Progress {
                task_id: "initialize".into(),
                message: "Resolving git branch...".into(),
                current: 1,
                total: 3,
            }));
            let branch = git::current_branch()
                .await
                .unwrap_or_else(|_| "(detached)".to_string());
            let _ = tx.send(Message::CurrentBranch(branch));
        };

        let myself = async {
            let _ = tx.send(Message::Progress(Progress {
                task_id: "initialize".into(),
                message: "Fetching Jira identity...".into(),
                current: 2,
                total: 3,
            }));
            let result = client
                .get_myself()
                .await
                .map(|u| u.account_id.unwrap_or_default());
            let _ = tx.send(Message::Myself(result));
        };

        let issues = async {
            let _ = tx.send(Message::Progress(Progress {
                task_id: "initialize".into(),
                message: "Fetching issues...".into(),
                current: 3,
                total: 3,
            }));
            let result = client.search(&jql).await;
            let _ = tx.send(Message::Issues(result));
        };

        tokio::join!(current_branch, myself, issues);
    });
}
