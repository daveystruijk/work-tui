//! **Fetch Children** — re-fetches child issues for a Jira parent issue.
//!
//! # Channel messages produced
//! - [`ActionMessage::TaskStarted`]
//! - [`ActionMessage::TaskFinished`]
//! - [`ActionMessage::ChildrenLoaded`]

use tokio::sync::mpsc;

use super::ActionMessage;
use crate::jira::JiraClient;

/// Spawn a Jira child issue fetch.
pub fn spawn(tx: mpsc::UnboundedSender<ActionMessage>, client: JiraClient, parent_key: String) {
    let _ = tx.send(ActionMessage::TaskStarted("Fetching children"));
    let tx = tx.clone();
    tokio::spawn(async move {
        let result = client
            .search(&format!("parent = {parent_key} ORDER BY created DESC"))
            .await;
        let _ = tx.send(ActionMessage::TaskFinished("Fetching children"));
        let _ = tx.send(ActionMessage::ChildrenLoaded(parent_key, result));
    });
}
