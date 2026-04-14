//! **Create Inline Issue** — creates a new Jira task from the inline editor.
//!
//! Fetches issue types for the project, picks the "Task" type, then creates
//! the issue via the Jira API. When a parent key is provided the new task is
//! linked as a child.
//!
//! # Channel messages produced
//! - [`ActionMessage::Progress`]
//! - [`ActionMessage::InlineCreated`]

use color_eyre::eyre::eyre;
use tokio::sync::mpsc;

use super::ActionMessage;
use crate::actions::Progress;
use crate::jira::JiraClient;

/// Spawn inline issue creation.
pub fn spawn(
    tx: mpsc::UnboundedSender<ActionMessage>,
    client: JiraClient,
    project_key: String,
    summary: String,
    parent_key: Option<String>,
) {
    let _ = tx.send(ActionMessage::TaskStarted("Creating issue"));
    let _ = tx.send(ActionMessage::Progress(Progress {
        action: "create_inline_issue",
        message: "Creating issue...".into(),
        current: 1,
        total: 1,
    }));

    tokio::spawn(async move {
        let result = async {
            let issue_types = client.get_issue_types(&project_key).await?;
            let issue_type = issue_types
                .iter()
                .find(|t| t.name.eq_ignore_ascii_case("task"))
                .or_else(|| issue_types.first())
                .ok_or_else(|| eyre!("No issue types found for project {project_key}"))?;

            client
                .create_issue(
                    &project_key,
                    &issue_type.id,
                    &summary,
                    None,
                    parent_key.as_deref(),
                )
                .await
        }
        .await;

        let _ = tx.send(ActionMessage::TaskFinished("Creating issue"));
        let _ = tx.send(ActionMessage::InlineCreated(result));
    });
}
