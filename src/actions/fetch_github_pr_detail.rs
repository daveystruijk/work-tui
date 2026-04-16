use tokio::sync::mpsc;

use super::ActionMessage;
use crate::github;

pub fn spawn(
    tx: mpsc::UnboundedSender<ActionMessage>,
    issue_key: String,
    repo_slug: String,
    pr_number: u64,
) {
    let _ = tx.send(ActionMessage::TaskStarted("Fetching PR detail"));
    let tx = tx.clone();
    tokio::spawn(async move {
        let result = github::fetch_pr_detail(&repo_slug, pr_number).await;
        let _ = tx.send(ActionMessage::TaskFinished("Fetching PR detail"));
        let _ = tx.send(ActionMessage::GithubPrDetail(issue_key, result));
    });
}
