//! **Refresh** — re-fetches the Jira issue list.
//!
//! This is the entry point for a full refresh cycle. Once issues arrive the
//! main loop chains further actions (branches, PRs, statuses).
//!
//! # Channel messages produced
//! - [`BgMsg::Progress`]
//! - [`BgMsg::Issues`]

use tokio::sync::mpsc;

use crate::actions::Progress;
use crate::app::BgMsg;
use crate::jira::JiraClient;

/// Spawn a Jira issue refresh.
pub fn spawn(tx: mpsc::UnboundedSender<BgMsg>, client: JiraClient, jql: String) {
    let _ = tx.send(BgMsg::Progress(Progress {
        action: "refresh",
        message: "Refreshing issues...".into(),
        current: 0,
        total: 0,
    }));
    tokio::spawn(async move {
        let result = client.search(&jql).await;
        let _ = tx.send(BgMsg::Issues(result));
    });
}
