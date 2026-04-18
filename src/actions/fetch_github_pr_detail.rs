use super::ActionMessage;
use crate::apis::github;

pub fn spawn(
    tx: tokio::sync::mpsc::UnboundedSender<ActionMessage>,
    issue_key: String,
    repo_slug: String,
    pr_number: u64,
) {
    super::spawn_action(tx, format!("Fetching PR detail for {issue_key}"), move |tx| async move {
        let result = github::fetch_pr_detail(&repo_slug, pr_number).await;
        let _ = tx.send(ActionMessage::GithubPrDetail(issue_key, result));
    });
}
