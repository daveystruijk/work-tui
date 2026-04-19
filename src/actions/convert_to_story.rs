//! **Convert issue type** — changes an issue's type to Story or Task.
//!
//! # Channel messages produced
//! - [`Message::ActionStarted`] / [`Message::ActionFinished`]
//! - [`Message::ConvertedToStory`]

use super::Message;
use crate::apis::jira::JiraClient;

pub fn spawn(
    tx: tokio::sync::mpsc::UnboundedSender<Message>,
    client: JiraClient,
    issue_key: String,
    target_type: &'static str,
) {
    super::spawn_action(
        tx,
        format!("convert_to_story:{issue_key}"),
        format!("Converting to {target_type}"),
        move |tx| async move {
            let result = client.update_issue_type(&issue_key, target_type).await;
            let _ = tx.send(Message::ConvertedToStory(issue_key, result));
        },
    );
}
