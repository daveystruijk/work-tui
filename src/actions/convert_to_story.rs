//! **Convert issue type** — changes an issue's type to Story or Task.
//!
//! # Channel messages produced
//! - [`ActionMessage::TaskStarted`] / [`ActionMessage::TaskFinished`]
//! - [`ActionMessage::ConvertedToStory`]

use super::ActionMessage;
use crate::apis::jira::JiraClient;

pub fn spawn(
    tx: tokio::sync::mpsc::UnboundedSender<ActionMessage>,
    client: JiraClient,
    issue_key: String,
    target_type: &'static str,
) {
    super::spawn_action(
        tx,
        format!("Converting to {target_type}"),
        move |tx| async move {
            let result = client.update_issue_type(&issue_key, target_type).await;
            let _ = tx.send(ActionMessage::ConvertedToStory(issue_key, result));
        },
    );
}
