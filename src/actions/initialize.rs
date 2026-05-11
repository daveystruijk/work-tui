//! **Initialize** — bootstraps the application on startup.
//!
//! Spawns three independent tasks in parallel:
//! 1. Resolve the current git branch
//! 2. Fetch the Jira user identity
//! 3. Fetch Jira projects for the filter UI
//!
//! # Channel messages produced
//! - [`Message::CurrentBranch`]
//! - [`Message::Myself`]
//! - [`Message::ProjectsLoaded`]
//! - [`Message::Progress`] (one per sub-task)

use tokio::sync::mpsc;

use super::Message;
use crate::actions::Progress;
use crate::apis::jira::JiraClient;
use crate::git;

/// Spawn all initialization tasks concurrently.
pub fn spawn(tx: mpsc::UnboundedSender<Message>, client: JiraClient) {
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

        let projects = async {
            let _ = tx.send(Message::Progress(Progress {
                task_id: "initialize".into(),
                message: "Fetching Jira projects...".into(),
                current: 3,
                total: 3,
            }));
            let result = client.get_projects().await;
            let _ = tx.send(Message::ProjectsLoaded(result));
        };

        tokio::join!(current_branch, myself, projects);
    });
}
