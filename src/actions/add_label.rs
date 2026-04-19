//! **Add Label** — adds a repo label to a Jira issue in the background.
//!
//! # Channel messages produced
//! - [`Message::ActionStarted`] / [`Message::ActionFinished`]
//! - [`Message::LabelAdded`]

use tokio::sync::mpsc;

use super::Message;
use crate::apis::jira::JiraClient;

/// Spawn label addition for a single issue.
pub fn spawn(
    tx: mpsc::UnboundedSender<Message>,
    client: JiraClient,
    issue_key: String,
    label: String,
    labels: Vec<String>,
) {
    super::spawn_action(tx, "add_label", "Adding label", |tx| async move {
        let mut labels = labels;
        labels.push(label.clone());
        let result = client
            .update_labels(&issue_key, &labels)
            .await
            .map(|_| (issue_key, label));
        let _ = tx.send(Message::LabelAdded(result));
    });
}
