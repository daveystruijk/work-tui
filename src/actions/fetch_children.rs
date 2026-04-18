//! **Fetch Children** — re-fetches child issues for a Jira parent issue.
//!
//! # Channel messages produced
//! - [`ActionMessage::TaskStarted`]
//! - [`ActionMessage::TaskFinished`]
//! - [`ActionMessage::ChildrenLoaded`]

use tokio::sync::mpsc;

use super::ActionMessage;
use crate::apis::jira::JiraClient;

/// Spawn a Jira child issue fetch.
pub fn spawn(tx: mpsc::UnboundedSender<ActionMessage>, client: JiraClient, parent_key: String) {
    tokio::spawn(async move {
        let task_name = format!("Fetching children for {parent_key}");
        let _ = tx.send(ActionMessage::TaskStarted(task_name.clone()));
        let result = run(&client, &parent_key).await;
        let _ = tx.send(ActionMessage::TaskFinished(task_name));
        let _ = tx.send(ActionMessage::ChildrenLoaded(parent_key, result));
    });
}

async fn run(client: &JiraClient, parent_key: &str) -> color_eyre::Result<Vec<crate::apis::jira::Issue>> {
    client
        .search(&format!("parent = {parent_key} ORDER BY created DESC"))
        .await
}
