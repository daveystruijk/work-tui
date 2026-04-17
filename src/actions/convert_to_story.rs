//! **Convert issue type** — changes an issue's type to Story or Task.
//!
//! # Channel messages produced
//! - [`ActionMessage::TaskStarted`] / [`ActionMessage::TaskFinished`]
//! - [`ActionMessage::ConvertedToStory`]

use tokio::sync::mpsc;

use super::ActionMessage;
use crate::apis::jira::JiraClient;

pub fn spawn(
    tx: mpsc::UnboundedSender<ActionMessage>,
    client: JiraClient,
    issue_key: String,
    target_type: &'static str,
) {
    tokio::spawn(async move {
        let label = format!("Converting to {target_type}");
        let _ = tx.send(ActionMessage::TaskStarted(label.clone()));
        let result = client.update_issue_type(&issue_key, target_type).await;
        let _ = tx.send(ActionMessage::TaskFinished(label));
        let _ = tx.send(ActionMessage::ConvertedToStory(issue_key, result));
    });
}
