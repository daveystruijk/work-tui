use tokio::sync::mpsc;

use super::ActionMessage;
use crate::apis::github;

pub fn spawn(
    tx: mpsc::UnboundedSender<ActionMessage>,
    issue_key: String,
    repo_slug: String,
    pr_number: u64,
) {
    tokio::spawn(async move {
        let task_name = format!("Fetching PR detail for {issue_key}");
        let _ = tx.send(ActionMessage::TaskStarted(task_name.clone()));
        let result = run(&repo_slug, pr_number).await;
        let _ = tx.send(ActionMessage::TaskFinished(task_name));
        let _ = tx.send(ActionMessage::GithubPrDetail(issue_key, result));
    });
}

async fn run(repo_slug: &str, pr_number: u64) -> color_eyre::Result<github::PrDetail> {
    github::fetch_pr_detail(repo_slug, pr_number).await
}
