//! **Import Tasks** — imports tasks from a `tasks.json` file in openspec changes.
//!
//! Discovers `$REPOS_DIR/openspec/changes/<issue-key-lowercased>-*/tasks.json`,
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
//! - [`Message::Progress`]
//! - [`Message::TasksImported`]

use std::path::{Path, PathBuf};

use color_eyre::{eyre::eyre, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use super::Message;
use crate::actions::Progress;
use crate::apis::jira::JiraClient;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TaskEntry {
    pub title: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
}

/// Resolve the openspec changes directory if it exists.
pub fn openspec_changes_dir(repos_dir: &Path) -> Option<PathBuf> {
    let path = repos_dir.join("openspec").join("changes");
    path.is_dir().then_some(path)
}

/// Find the tasks.json path for the given issue key under openspec changes.
pub fn find_tasks_json(repos_dir: &Path, issue_key: &str) -> Result<PathBuf> {
    let Some(changes_dir) = openspec_changes_dir(repos_dir) else {
        return Err(eyre!(
            "No openspec/changes directory found under {}",
            repos_dir.display()
        ));
    };

    let prefix = format!("{}-", issue_key.to_lowercase());
    let entries = std::fs::read_dir(&changes_dir)
        .map_err(|err| eyre!("Failed to read {}: {err}", changes_dir.display()))?;

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

/// Load and parse tasks from a tasks.json file.
pub fn load_tasks(path: &Path) -> Result<Vec<TaskEntry>> {
    let content = std::fs::read_to_string(path)
        .map_err(|err| eyre!("Failed to read {}: {err}", path.display()))?;
    let tasks: Vec<TaskEntry> =
        serde_json::from_str(&content).map_err(|err| eyre!("Failed to parse tasks.json: {err}"))?;
    Ok(tasks)
}

/// Spawn the import tasks action.
pub fn spawn(
    tx: mpsc::UnboundedSender<Message>,
    client: JiraClient,
    tasks_path: PathBuf,
    tasks: Vec<TaskEntry>,
    issue_key: String,
    issue_type_name: String,
    project_key: String,
) {
    super::spawn_action(tx, "import_tasks", "Importing tasks", |tx| async move {
        let result = run(
            &tx,
            &client,
            &tasks_path,
            tasks,
            &issue_key,
            &issue_type_name,
            &project_key,
        )
        .await;
        let _ = tx.send(Message::TasksImported(issue_key, result));
    });
}

async fn run(
    tx: &mpsc::UnboundedSender<Message>,
    client: &JiraClient,
    tasks_path: &Path,
    mut tasks: Vec<TaskEntry>,
    issue_key: &str,
    issue_type_name: &str,
    project_key: &str,
) -> Result<()> {
    let pending_tasks: Vec<usize> = tasks
        .iter()
        .enumerate()
        .filter(|(_, task)| task.key.is_none())
        .map(|(index, _)| index)
        .collect();

    if pending_tasks.is_empty() {
        return Err(eyre!("All tasks already have keys assigned"));
    }

    let pending_task_count = pending_tasks.len();
    let base_progress_total = pending_task_count + 1;

    if pending_task_count == 1 {
        let task_index = pending_tasks[0];
        let task = &tasks[task_index];

        let _ = tx.send(Message::Progress(Progress {
            task_id: "import_tasks".into(),
            message: format!("Updating {issue_key}..."),
            current: 1,
            total: base_progress_total,
        }));

        client
            .append_description(issue_key, &task.description)
            .await?;

        tasks[task_index].key = Some(issue_key.to_string());
        write_tasks_json(tasks_path, &tasks)?;

        return Ok(());
    }

    // Multiple tasks: ensure the issue is a Story first
    let issue_type_name = issue_type_name.to_lowercase();
    let is_story = issue_type_name.contains("story") || issue_type_name.contains("epic");
    let multi_task_progress_total = base_progress_total + 1;
    let first_create_step = 2;

    if !is_story {
        let _ = tx.send(Message::Progress(Progress {
            task_id: "import_tasks".into(),
            message: format!("Converting {issue_key} to Story..."),
            current: 1,
            total: multi_task_progress_total,
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

        let _ = tx.send(Message::Progress(Progress {
            task_id: "import_tasks".into(),
            message: format!("Creating subtask: {}...", task.title),
            current: step + first_create_step,
            total: multi_task_progress_total,
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
        write_tasks_json(tasks_path, &tasks)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TestReposDir {
        path: PathBuf,
    }

    impl TestReposDir {
        fn new(name: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir()
                .join(format!("work-tui-{name}-{}-{unique}", std::process::id()));
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }
    }

    impl Drop for TestReposDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn finds_tasks_json_in_openspec_changes() {
        let repos_dir = TestReposDir::new("finds-openspec-tasks-json");
        let tasks_path = repos_dir
            .path
            .join("openspec")
            .join("changes")
            .join("ini-4347-add-note")
            .join("tasks.json");

        fs::create_dir_all(tasks_path.parent().unwrap()).unwrap();
        fs::write(&tasks_path, "[]\n").unwrap();

        let found_path = find_tasks_json(&repos_dir.path, "INI-4347").unwrap();

        assert_eq!(found_path, tasks_path);
    }

    #[test]
    fn does_not_accept_opencode_changes() {
        let repos_dir = TestReposDir::new("rejects-opencode-tasks-json");

        let tasks_path = repos_dir
            .path
            .join("opencode")
            .join("changes")
            .join("ini-4347-add-note")
            .join("tasks.json");

        fs::create_dir_all(tasks_path.parent().unwrap()).unwrap();
        fs::write(&tasks_path, "[]\n").unwrap();

        let err = find_tasks_json(&repos_dir.path, "INI-4347").unwrap_err();

        assert!(err
            .to_string()
            .contains("No openspec/changes directory found"));
    }
}
