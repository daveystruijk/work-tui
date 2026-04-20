//! **Create Inline Issue** — creates a new Jira task from the inline editor.
//!
//! Fetches issue types for the project, picks the "Task" type, then creates
//! the issue via the Jira API. When a parent key is provided the new task is
//! linked as a child.
//!
//! # Channel messages produced
//! - [`Message::Progress`]
//! - [`Message::InlineCreated`]

use std::time::Duration;

use color_eyre::eyre::eyre;
use tokio::sync::mpsc;
use tokio::time::sleep;

use super::Message;
use crate::actions::Progress;
use crate::apis::jira::Issue;
use crate::apis::jira::JiraClient;

struct RunResult {
    created_key: String,
    refreshed_issues: Option<Vec<Issue>>,
}

/// Spawn inline issue creation.
pub fn spawn(
    tx: mpsc::UnboundedSender<Message>,
    client: JiraClient,
    jql: String,
    project_key: String,
    summary: String,
    parent_key: Option<String>,
) {
    super::spawn_action(
        tx,
        "create_inline_issue",
        "Creating issue",
        |tx| async move {
            let _ = tx.send(Message::Progress(Progress {
                task_id: "create_inline_issue".into(),
                message: "Creating issue...".into(),
                current: 1,
                total: 1,
            }));
            let result: color_eyre::Result<RunResult> = async {
                let issue_types = client.get_issue_types(&project_key).await?;
                let issue_type = if parent_key.is_some() {
                    issue_types
                        .iter()
                        .find(|t| t.is_subtask())
                        .ok_or_else(|| eyre!("No subtask type found for project {project_key}"))?
                } else {
                    issue_types
                        .iter()
                        .find(|t| t.is_standard() && t.name.eq_ignore_ascii_case("task"))
                        .or_else(|| issue_types.iter().find(|t| t.is_standard()))
                        .ok_or_else(|| eyre!("No issue types found for project {project_key}"))?
                };

                let created_key = client
                    .create_issue(
                        &project_key,
                        &issue_type.id,
                        &summary,
                        None,
                        parent_key.as_deref(),
                    )
                    .await?;

                let mut last_issues = None;
                for attempt in 0..6 {
                    if let Ok(issues) = client.search(&jql).await {
                        if issues.iter().any(|issue| issue.key == created_key) {
                            return Ok(RunResult {
                                created_key,
                                refreshed_issues: Some(issues),
                            });
                        }
                        last_issues = Some(issues);
                    }

                    if attempt < 5 {
                        let _ = tx.send(Message::Progress(Progress {
                            task_id: "create_inline_issue".into(),
                            message: format!("Waiting for Jira to index {created_key}..."),
                            current: attempt + 2,
                            total: 7,
                        }));
                        sleep(Duration::from_millis(250)).await;
                    }
                }

                Ok(RunResult {
                    created_key,
                    refreshed_issues: last_issues,
                })
            }
            .await;
            match result {
                Ok(run_result) => {
                    if let Some(issues) = run_result.refreshed_issues {
                        let _ = tx.send(Message::Issues(Ok(issues)));
                    }
                    let _ = tx.send(Message::InlineCreated(Ok(run_result.created_key)));
                }
                Err(error) => {
                    let _ = tx.send(Message::InlineCreated(Err(error)));
                }
            }
        },
    );
}
