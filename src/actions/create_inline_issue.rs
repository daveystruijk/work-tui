//! **Create Inline Issue** — creates a new Jira subtask from the inline editor.
//!
//! Submits the summary typed in the inline-new row as a new task (or subtask
//! when a parent key is provided) via the Jira API.
//!
//! # Channel messages produced
//! - [`BgMsg::Progress`]
//! - [`BgMsg::InlineCreated`]

use tokio::sync::mpsc;

use crate::actions::Progress;
use crate::app::BgMsg;
use crate::jira::JiraClient;

/// Spawn inline issue creation.
pub fn spawn(
    tx: mpsc::UnboundedSender<BgMsg>,
    client: JiraClient,
    project_key: String,
    summary: String,
    parent_key: Option<String>,
) {
    let _ = tx.send(BgMsg::TaskStarted("Creating issue"));
    let _ = tx.send(BgMsg::Progress(Progress {
        action: "create_inline_issue",
        message: "Creating issue...".into(),
        current: 1,
        total: 1,
    }));

    tokio::spawn(async move {
        let result = client
            .create_issue(&project_key, "10001", &summary, None, parent_key.as_deref())
            .await;
        let _ = tx.send(BgMsg::TaskFinished("Creating issue"));
        let _ = tx.send(BgMsg::InlineCreated(result));
    });
}
