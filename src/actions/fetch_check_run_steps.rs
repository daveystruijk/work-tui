use super::Message;
use crate::apis::github::{self, CheckRun};

pub fn spawn(
    tx: tokio::sync::mpsc::UnboundedSender<Message>,
    issue_key: String,
    repo_slug: String,
    check_runs: Vec<CheckRun>,
) {
    super::spawn_action(
        tx,
        format!("fetch_check_run_steps:{issue_key}"),
        format!("Fetching CI steps for #{issue_key}"),
        move |tx| async move {
            let result = github::fetch_check_run_steps(&repo_slug, &check_runs).await;
            let _ = tx.send(Message::CheckRunSteps(issue_key, result));
        },
    );
}
