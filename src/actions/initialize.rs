//! **Initialize** — bootstraps the application on startup.
//!
//! Spawns three independent tasks in parallel:
//! 1. Resolve the current git branch
//! 2. Fetch the Jira user identity
//! 3. Fetch the initial issue list
//!
//! # Channel messages produced
//! - [`BgMsg::CurrentBranch`]
//! - [`BgMsg::Myself`]
//! - [`BgMsg::Issues`]
//! - [`BgMsg::Progress`] (one per sub-task)

use tokio::sync::mpsc;

use crate::actions::Progress;
use crate::app::BgMsg;
use crate::git;
use crate::jira::JiraClient;

/// Spawn all initialization tasks concurrently.
pub fn spawn(tx: mpsc::UnboundedSender<BgMsg>, client: JiraClient, jql: String) {
    // 1. Resolve current git branch
    {
        let tx = tx.clone();
        tokio::spawn(async move {
            let _ = tx.send(BgMsg::Progress(Progress {
                action: "initialize",
                message: "Resolving git branch...".into(),
                current: 1,
                total: 3,
            }));
            let branch = git::current_branch()
                .await
                .unwrap_or_else(|_| "(detached)".to_string());
            let _ = tx.send(BgMsg::CurrentBranch(branch));
        });
    }

    // 2. Fetch Jira user identity
    {
        let tx = tx.clone();
        let client = client.clone();
        tokio::spawn(async move {
            let _ = tx.send(BgMsg::Progress(Progress {
                action: "initialize",
                message: "Fetching Jira identity...".into(),
                current: 2,
                total: 3,
            }));
            let result = client
                .get_myself()
                .await
                .map(|u| u.account_id.unwrap_or_default());
            let _ = tx.send(BgMsg::Myself(result));
        });
    }

    // 3. Fetch issues
    {
        let tx = tx.clone();
        tokio::spawn(async move {
            let _ = tx.send(BgMsg::Progress(Progress {
                action: "initialize",
                message: "Fetching issues...".into(),
                current: 3,
                total: 3,
            }));
            let result = client.search(&jql).await;
            let _ = tx.send(BgMsg::Issues(result));
        });
    }
}
