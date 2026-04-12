//! **Initialize** — bootstraps the application on startup.
//!
//! Spawns three independent tasks in parallel:
//! 1. Resolve the current git branch
//! 2. Fetch the Jira user identity
//! 3. Fetch the initial issue list
//!
//! # Channel messages produced
//! - [`ActionMessage::CurrentBranch`]
//! - [`ActionMessage::Myself`]
//! - [`ActionMessage::Issues`]
//! - [`ActionMessage::Progress`] (one per sub-task)

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use tokio::sync::mpsc;

use super::ActionMessage;
use crate::actions::Progress;
use crate::git;
use crate::jira::JiraClient;

/// Spawn all initialization tasks concurrently.
pub fn spawn(tx: mpsc::UnboundedSender<ActionMessage>, client: JiraClient, jql: String) {
    let _ = tx.send(ActionMessage::TaskStarted("Initializing"));
    let done = Arc::new(AtomicUsize::new(0));

    // 1. Resolve current git branch
    {
        let tx = tx.clone();
        let done = Arc::clone(&done);
        tokio::spawn(async move {
            let _ = tx.send(ActionMessage::Progress(Progress {
                action: "initialize",
                message: "Resolving git branch...".into(),
                current: 1,
                total: 3,
            }));
            let branch = git::current_branch()
                .await
                .unwrap_or_else(|_| "(detached)".to_string());
            let _ = tx.send(ActionMessage::CurrentBranch(branch));
            if done.fetch_add(1, Ordering::Relaxed) == 2 {
                let _ = tx.send(ActionMessage::TaskFinished("Initializing"));
            }
        });
    }

    // 2. Fetch Jira user identity
    {
        let tx = tx.clone();
        let client = client.clone();
        let done = Arc::clone(&done);
        tokio::spawn(async move {
            let _ = tx.send(ActionMessage::Progress(Progress {
                action: "initialize",
                message: "Fetching Jira identity...".into(),
                current: 2,
                total: 3,
            }));
            let result = client
                .get_myself()
                .await
                .map(|u| u.account_id.unwrap_or_default());
            let _ = tx.send(ActionMessage::Myself(result));
            if done.fetch_add(1, Ordering::Relaxed) == 2 {
                let _ = tx.send(ActionMessage::TaskFinished("Initializing"));
            }
        });
    }

    // 3. Fetch issues
    {
        let tx = tx.clone();
        let done = Arc::clone(&done);
        tokio::spawn(async move {
            let _ = tx.send(ActionMessage::Progress(Progress {
                action: "initialize",
                message: "Fetching issues...".into(),
                current: 3,
                total: 3,
            }));
            let result = client.search(&jql).await;
            let _ = tx.send(ActionMessage::Issues(result));
            if done.fetch_add(1, Ordering::Relaxed) == 2 {
                let _ = tx.send(ActionMessage::TaskFinished("Initializing"));
            }
        });
    }
}
