//! **Add Label** — adds a repo label to a Jira issue in the background.
//!
//! # Channel messages produced
//! - [`ActionMessage::TaskStarted`] / [`ActionMessage::TaskFinished`]
//! - [`ActionMessage::LabelAdded`]

use tokio::sync::mpsc;

use super::ActionMessage;
use crate::apis::jira::JiraClient;

/// Spawn label addition for a single issue.
pub fn spawn(
    tx: mpsc::UnboundedSender<ActionMessage>,
    client: JiraClient,
    issue_key: String,
    label: String,
    mut labels: Vec<String>,
) {
    let _ = tx.send(ActionMessage::TaskStarted("Adding label".to_string()));
    tokio::spawn(async move {
        labels.push(label.clone());
        let result = client
            .update_labels(&issue_key, &labels)
            .await
            .map(|_| (issue_key, label));
        let _ = tx.send(ActionMessage::TaskFinished("Adding label".to_string()));
        let _ = tx.send(ActionMessage::LabelAdded(result));
    });
}
