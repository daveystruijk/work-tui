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
use crate::apis::jira::Issue;
use crate::apis::jira::JiraClient;

struct RunResult {
    created_key: String,
    refreshed_issues: Option<Vec<Issue>>,
}

/// Spawn inline issue creation.
pub fn spawn(
    tx: mpsc::UnboundedSender<ActionMessage>,
    client: JiraClient,
    jql: String,
    project_key: String,
    summary: String,
    parent_key: Option<String>,
) {
    tokio::spawn(async move {
        let _ = tx.send(ActionMessage::TaskStarted("Creating issue".to_string()));
        let _ = tx.send(ActionMessage::Progress(Progress {
            action: "create_inline_issue",
            message: "Creating issue...".into(),
            current: 1,
            total: 1,
        }));
        let result = run(&tx, &client, &project_key, &summary, parent_key.as_deref(), &jql).await;
        let _ = tx.send(ActionMessage::TaskFinished("Creating issue".to_string()));
        match result {
            Ok(result) => {
                if let Some(issues) = result.refreshed_issues {
                    let _ = tx.send(ActionMessage::Issues(Ok(issues)));
                }
                let _ = tx.send(ActionMessage::InlineCreated(Ok(result.created_key)));
            }
            Err(error) => {
                let _ = tx.send(ActionMessage::InlineCreated(Err(error)));
            }
        }
    });
}

async fn run(
    tx: &mpsc::UnboundedSender<ActionMessage>,
    client: &JiraClient,
    project_key: &str,
    summary: &str,
    parent_key: Option<&str>,
    jql: &str,
) -> color_eyre::Result<RunResult> {
    let issue_types = client.get_issue_types(project_key).await?;
    let issue_type = if parent_key.is_some() {
        issue_types
            .iter()
            .find(|t| t.subtask)
            .ok_or_else(|| eyre!("No subtask type found for project {project_key}"))?
    } else {
        issue_types
            .iter()
            .find(|t| !t.subtask && t.name.eq_ignore_ascii_case("task"))
            .or_else(|| issue_types.iter().find(|t| !t.subtask))
            .ok_or_else(|| eyre!("No issue types found for project {project_key}"))?
    };

    let created_key = client
        .create_issue(project_key, &issue_type.id, summary, None, parent_key)
        .await?;

    let mut last_issues = None;
    for attempt in 0..6 {
        if let Ok(issues) = client.search(jql).await {
            if issues.iter().any(|issue| issue.key == created_key) {
                return Ok(RunResult { created_key, refreshed_issues: Some(issues) });
            }
            last_issues = Some(issues);
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

    Ok(RunResult { created_key, refreshed_issues: last_issues })
}
