//! **Fetch Children** — re-fetches child issues for a Jira parent issue.
//!
//! # Channel messages produced
//! - [`ActionMessage::TaskStarted`]
//! - [`ActionMessage::TaskFinished`]
//! - [`ActionMessage::ChildrenLoaded`]

use tokio::sync::mpsc;

use super::ActionMessage;
use crate::apis::jira::JiraClient;

/// Spawn a Jira child issue fetch.
pub fn spawn(tx: mpsc::UnboundedSender<ActionMessage>, client: JiraClient, parent_key: String) {
    super::spawn_action(
        tx,
        format!("Fetching children for {parent_key}"),
        |tx| async move {
            let result = client
                .search(&format!("parent = {parent_key} ORDER BY created DESC"))
                .await;
            let _ = tx.send(ActionMessage::ChildrenLoaded(parent_key, result));
        },
    );
}
