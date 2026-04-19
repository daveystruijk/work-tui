//! **Fetch Children** — re-fetches child issues for a Jira parent issue.
//!
//! # Channel messages produced
//! - [`Message::ActionStarted`]
//! - [`Message::ActionFinished`]
//! - [`Message::ChildrenLoaded`]

use tokio::sync::mpsc;

use super::Message;
use crate::apis::jira::JiraClient;

/// Spawn a Jira child issue fetch.
pub fn spawn(tx: mpsc::UnboundedSender<Message>, client: JiraClient, parent_key: String) {
    super::spawn_action(
        tx,
        format!("fetch_children:{parent_key}"),
        format!("Fetching children for {parent_key}"),
        |tx| async move {
            let result = client
                .search(&format!("parent = {parent_key} ORDER BY created DESC"))
                .await;
            let _ = tx.send(Message::ChildrenLoaded(parent_key, result));
        },
    );
}
