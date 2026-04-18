//! **Import Tasks** — imports tasks from a `tasks.json` file in the opencode directory.
//!
//! Discovers `$REPOS_DIR/opencode/changes/<issue-key-lowercased>-*/tasks.json`,
//! reads the task list, and creates Jira subtasks under the current issue.
//!
//! If there is only one task, updates the current issue directly instead of
//! creating a subtask. If the issue is not already a Story and there are
//! multiple tasks, converts it to a Story first.
//!
//! Appends the task description to the Jira issue description (preserving any
//! existing description). After creating each task, writes back the Jira key
//! into the `tasks.json` file. Tasks that already have a `"key"` field are
//! skipped.
//!
//! # Channel messages produced
//! - [`ActionMessage::Progress`]
//! - [`ActionMessage::TasksImported`]

use std::path::{Path, PathBuf};

use color_eyre::{eyre::eyre, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use super::ActionMessage;
use crate::actions::Progress;
use crate::apis::jira::JiraClient;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TaskEntry {
    pub title: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
}

/// Find the tasks.json path for the given issue key under `$REPOS_DIR/opencode/changes/`.
fn find_tasks_json(repos_dir: &Path, issue_key: &str) -> Result<PathBuf> {
    let changes_dir = repos_dir.join("opencode").join("changes");
    if !changes_dir.is_dir() {
        return Err(eyre!(
            "Changes directory does not exist: {}",
            changes_dir.display()
        ));
    }

    let prefix = format!("{}-", issue_key.to_lowercase());
    let entries =
        std::fs::read_dir(&changes_dir).map_err(|err| eyre!("Failed to read changes dir: {err}"))?;

    for entry in entries {
        let entry = entry.map_err(|err| eyre!("Failed to read entry: {err}"))?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|os| os.to_str()) else {
            continue;
        };
        if !name.starts_with(&prefix) {
            continue;
        }
        let tasks_path = path.join("tasks.json");
        if tasks_path.exists() {
            return Ok(tasks_path);
        }
    }

    Err(eyre!(
        "No tasks.json found for {issue_key} in {}",
        changes_dir.display()
    ))
}

/// Spawn the import tasks action.
pub fn spawn(
    tx: mpsc::UnboundedSender<ActionMessage>,
    client: JiraClient,
    repos_dir: PathBuf,
    issue_key: String,
    issue_type_name: String,
    project_key: String,
) {
    super::spawn_action(tx, "Importing tasks", |tx| async move {
        let result = run(
            &tx,
            &client,
            &repos_dir,
            &issue_key,
            &issue_type_name,
            &project_key,
        )
        .await;
        let _ = tx.send(ActionMessage::TasksImported(issue_key, result));
    });
}

async fn run(
    tx: &mpsc::UnboundedSender<ActionMessage>,
    client: &JiraClient,
    repos_dir: &Path,
    issue_key: &str,
    issue_type_name: &str,
    project_key: &str,
) -> Result<()> {
    let _ = tx.send(ActionMessage::Progress(Progress {
        action: "import_tasks",
        message: "Finding tasks.json...".into(),
        current: 1,
        total: 0,
    }));

    let tasks_path = find_tasks_json(repos_dir, issue_key)?;
    let content = std::fs::read_to_string(&tasks_path)
        .map_err(|err| eyre!("Failed to read {}: {err}", tasks_path.display()))?;
    let mut tasks: Vec<TaskEntry> =
        serde_json::from_str(&content).map_err(|err| eyre!("Failed to parse tasks.json: {err}"))?;

    let pending_tasks: Vec<usize> = tasks
        .iter()
        .enumerate()
        .filter(|(_, task)| task.key.is_none())
        .map(|(index, _)| index)
        .collect();

    if pending_tasks.is_empty() {
        return Err(eyre!("All tasks already have keys assigned"));
    }

    let total_steps = pending_tasks.len() + 1; // +1 for setup

    if pending_tasks.len() == 1 {
        // Single task: update the current issue directly
        let task_index = pending_tasks[0];
        let task = &tasks[task_index];

        let _ = tx.send(ActionMessage::Progress(Progress {
            action: "import_tasks",
            message: format!("Updating {issue_key}..."),
            current: 2,
            total: total_steps,
        }));

        client
            .append_description(issue_key, &task.description)
            .await?;

        tasks[task_index].key = Some(issue_key.to_string());
        write_tasks_json(&tasks_path, &tasks)?;

        return Ok(());
    }

    // Multiple tasks: ensure the issue is a Story first
    let is_story = issue_type_name.to_lowercase().contains("story")
        || issue_type_name.to_lowercase().contains("epic");

    if !is_story {
        let _ = tx.send(ActionMessage::Progress(Progress {
            action: "import_tasks",
            message: format!("Converting {issue_key} to Story..."),
            current: 2,
            total: total_steps + 1,
        }));
        client.update_issue_type(issue_key, "Story").await?;
    }

    // Get subtask issue type
    let issue_types = client.get_issue_types(project_key).await?;
    let subtask_type = issue_types
        .iter()
        .find(|t| t.subtask)
        .ok_or_else(|| eyre!("No subtask type found for project {project_key}"))?;
    let subtask_type_id = subtask_type.id.clone();

    for (step, &task_index) in pending_tasks.iter().enumerate() {
        let task = &tasks[task_index];

        let _ = tx.send(ActionMessage::Progress(Progress {
            action: "import_tasks",
            message: format!("Creating subtask: {}...", task.title),
            current: step + 3,
            total: total_steps + 2,
        }));

        let created_key = client
            .create_issue(
                project_key,
                &subtask_type_id,
                &task.title,
                Some(&task.description),
                Some(issue_key),
            )
            .await?;

        tasks[task_index].key = Some(created_key);
        write_tasks_json(&tasks_path, &tasks)?;
    }

    Ok(())
}

fn write_tasks_json(path: &Path, tasks: &[TaskEntry]) -> Result<()> {
    let json = serde_json::to_string_pretty(tasks)
        .map_err(|err| eyre!("Failed to serialize tasks: {err}"))?;
    std::fs::write(path, format!("{json}\n"))
        .map_err(|err| eyre!("Failed to write {}: {err}", path.display()))?;
    Ok(())
}
