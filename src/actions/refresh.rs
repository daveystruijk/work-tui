//! **Refresh** — re-fetches the Jira issue list.
//!
//! This is the entry point for a full refresh cycle. Once issues arrive the
//! main loop chains further actions (branches, PRs, statuses).
//!
//! # Channel messages produced
//! - [`Message::Progress`]
//! - [`Message::Issues`]

use tokio::sync::mpsc;

use super::Message;
use crate::apis::jira::JiraClient;

/// Spawn a Jira issue refresh.
pub fn spawn(tx: mpsc::UnboundedSender<Message>, client: JiraClient, jql: String) {
    super::spawn_action(tx, "Refreshing issues", |tx| async move {
        let result = client.search(&jql).await;
        let _ = tx.send(Message::Issues(result));
    });
}
