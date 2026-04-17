//! **Convert to Story** — changes an issue's type to Story.
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
) {
    tokio::spawn(async move {
        let _ = tx.send(ActionMessage::TaskStarted("Converting to story".to_string()));
        let result = client.update_issue_type(&issue_key, "Story").await;
        let _ = tx.send(ActionMessage::TaskFinished("Converting to story".to_string()));
        let _ = tx.send(ActionMessage::ConvertedToStory(issue_key, result));
    });
}
