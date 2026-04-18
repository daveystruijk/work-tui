//! **Fetch CI Logs** — fetches check run logs on demand (when 'c' is pressed).
//!
//! # Channel messages produced
//! - [`ActionMessage::CiLogsFetched`]

use super::ActionMessage;
use crate::apis::github::{fetch_check_run_logs, CheckRun};

pub fn spawn(
    tx: tokio::sync::mpsc::UnboundedSender<ActionMessage>,
    issue_key: String,
    repo_slug: String,
    check_runs: Vec<CheckRun>,
) {
    super::spawn_action(tx, "Fetching CI logs", |tx| async move {
        let result = fetch_check_run_logs(&repo_slug, &check_runs).await;
        let _ = tx.send(ActionMessage::CiLogsFetched(issue_key, result));
    });
}
