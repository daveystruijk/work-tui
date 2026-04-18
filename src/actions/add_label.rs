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
    labels: Vec<String>,
) {
    tokio::spawn(async move {
        let _ = tx.send(ActionMessage::TaskStarted("Adding label".to_string()));
        let result = run(&client, &issue_key, &label, labels).await;
        let _ = tx.send(ActionMessage::TaskFinished("Adding label".to_string()));
        let _ = tx.send(ActionMessage::LabelAdded(result));
    });
}

async fn run(
    client: &JiraClient,
    issue_key: &str,
    label: &str,
    mut labels: Vec<String>,
) -> color_eyre::Result<(String, String)> {
    labels.push(label.to_string());
    client
        .update_labels(issue_key, &labels)
        .await
        .map(|_| (issue_key.to_string(), label.to_string()))
}
