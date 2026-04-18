//! **Refresh** — re-fetches the Jira issue list.
//!
//! This is the entry point for a full refresh cycle. Once issues arrive the
//! main loop chains further actions (branches, PRs, statuses).
//!
//! # Channel messages produced
//! - [`ActionMessage::Progress`]
//! - [`ActionMessage::Issues`]

use tokio::sync::mpsc;

use super::ActionMessage;
use crate::apis::jira::JiraClient;

/// Spawn a Jira issue refresh.
pub fn spawn(tx: mpsc::UnboundedSender<ActionMessage>, client: JiraClient, jql: String) {
    tokio::spawn(async move {
        let _ = tx.send(ActionMessage::TaskStarted("Refreshing issues".to_string()));
        let result = run(&client, &jql).await;
        let _ = tx.send(ActionMessage::TaskFinished("Refreshing issues".to_string()));
        let _ = tx.send(ActionMessage::Issues(result));
    });
}

async fn run(client: &JiraClient, jql: &str) -> color_eyre::Result<Vec<crate::apis::jira::Issue>> {
    client.search(jql).await
}
