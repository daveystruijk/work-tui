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
    super::spawn_action(tx, "Refreshing issues", |tx| async move {
        let result = client.search(&jql).await;
        let _ = tx.send(ActionMessage::Issues(result));
    });
}
