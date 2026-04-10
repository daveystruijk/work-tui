//! **Add Label** — adds a repo label to a Jira issue in the background.
//!
//! # Channel messages produced
//! - [`BgMsg::TaskStarted`] / [`BgMsg::TaskFinished`]
//! - [`BgMsg::LabelAdded`]

use tokio::sync::mpsc;

use crate::app::BgMsg;
use crate::jira::JiraClient;

/// Spawn label addition for a single issue.
pub fn spawn(
    tx: mpsc::UnboundedSender<BgMsg>,
    client: JiraClient,
    issue_key: String,
    label: String,
    mut labels: Vec<String>,
) {
    let _ = tx.send(BgMsg::TaskStarted("Adding label"));
    tokio::spawn(async move {
        labels.push(label.clone());
        let result = client
            .update_labels(&issue_key, &labels)
            .await
            .map(|_| (issue_key, label));
        let _ = tx.send(BgMsg::TaskFinished("Adding label"));
        let _ = tx.send(BgMsg::LabelAdded(result));
    });
}
