use tokio::sync::mpsc;

use super::Message;
use crate::apis::jira::JiraClient;

pub fn spawn_project_statuses(
    tx: mpsc::UnboundedSender<Message>,
    client: JiraClient,
    project_key: String,
) {
    super::spawn_action(
        tx,
        format!("fetch_project_statuses:{project_key}"),
        format!("Fetching Jira statuses for {project_key}"),
        |tx| async move {
            let result = client.get_project_statuses(&project_key).await;
            let _ = tx.send(Message::ProjectStatusesLoaded(project_key, result));
        },
    );
}
