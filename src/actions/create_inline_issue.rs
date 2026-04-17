//! **Create Inline Issue** — creates a new Jira task from the inline editor.
//!
//! Fetches issue types for the project, picks the "Task" type, then creates
//! the issue via the Jira API. When a parent key is provided the new task is
//! linked as a child.
//!
//! # Channel messages produced
//! - [`ActionMessage::Progress`]
//! - [`ActionMessage::InlineCreated`]

use std::time::Duration;

use color_eyre::eyre::eyre;
use tokio::sync::mpsc;
use tokio::time::sleep;

use super::ActionMessage;
use crate::actions::Progress;
use crate::apis::jira::JiraClient;

/// Spawn inline issue creation.
pub fn spawn(
    tx: mpsc::UnboundedSender<ActionMessage>,
    client: JiraClient,
    jql: String,
    project_key: String,
    summary: String,
    parent_key: Option<String>,
) {
    let _ = tx.send(ActionMessage::TaskStarted("Creating issue".to_string()));
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

        let _ = tx.send(ActionMessage::TaskFinished("Creating issue".to_string()));

        let Ok(created_key) = result else {
            let _ = tx.send(ActionMessage::InlineCreated(result));
            return;
        };

        let mut last_issues = None;
        for attempt in 0..6 {
            let issues_result = client.search(&jql).await;
            if let Ok(issues) = issues_result {
                let contains_created_key = issues.iter().any(|issue| issue.key == created_key);
                last_issues = Some(issues);
                if contains_created_key {
                    let _ = tx.send(ActionMessage::Issues(Ok(last_issues.take().unwrap())));
                    let _ = tx.send(ActionMessage::InlineCreated(Ok(created_key)));
                    return;
                }
            }

            if attempt < 5 {
                let _ = tx.send(ActionMessage::Progress(Progress {
                    action: "create_inline_issue",
                    message: format!("Waiting for Jira to index {created_key}..."),
                    current: attempt + 2,
                    total: 7,
                }));
                sleep(Duration::from_millis(250)).await;
            }
        }

        if let Some(issues) = last_issues {
            let _ = tx.send(ActionMessage::Issues(Ok(issues)));
        }
        let _ = tx.send(ActionMessage::InlineCreated(Ok(created_key)));
    });
}
