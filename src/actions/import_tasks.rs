//! **Import Tasks** — imports tasks from a `tasks.json` file in openspec changes.
//!
//! Discovers `$REPOS_DIR/openspec/changes/<issue-key-lowercased>-*/tasks.json`,
//! reads the task list, and creates Jira child issues under the current issue.
//!
//! If there is only one task, updates the current issue directly instead of
//! creating a child issue. If there are multiple tasks and the current issue is
//! an Epic, creates standard tasks beneath it. Otherwise, creates subtasks and
//! converts non-Story parents to Story first.
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
use crate::apis::jira::{IssueType, JiraClient};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TaskEntry {
    pub title: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// Title at the time of last import, used to detect changes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub imported_title: Option<String>,
    /// Description at the time of last import, used to detect changes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub imported_description: Option<String>,
}

impl TaskEntry {
    /// Returns true if this task was previously imported but the title or
    /// description has since changed in the openspec JSON.
    pub fn has_changes(&self) -> bool {
        let Some(ref _key) = self.key else {
            return false;
        };
        let title_changed = self
            .imported_title
            .as_ref()
            .map(|imported| imported != &self.title)
            .unwrap_or(false);
        let description_changed = self
            .imported_description
            .as_ref()
            .map(|imported| imported != &self.description)
            .unwrap_or(false);
        title_changed || description_changed
    }

    /// Returns true if this task needs action (either new or changed).
    pub fn needs_action(&self) -> bool {
        self.key.is_none() || self.has_changes()
    }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChildCreationMode {
    StandardTask,
    Subtask,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MultiTaskImportPlan {
    child_creation_mode: ChildCreationMode,
    should_convert_parent_to_story: bool,
}

fn plan_multi_task_import(issue_type_name: &str) -> MultiTaskImportPlan {
    let issue_type_name = issue_type_name.to_lowercase();

    if issue_type_name.contains("epic") {
        return MultiTaskImportPlan {
            child_creation_mode: ChildCreationMode::StandardTask,
            should_convert_parent_to_story: false,
        };
    }

    MultiTaskImportPlan {
        child_creation_mode: ChildCreationMode::Subtask,
        should_convert_parent_to_story: !issue_type_name.contains("story"),
    }
}

fn select_child_issue_type<'a>(
    issue_types: &'a [IssueType],
    child_creation_mode: ChildCreationMode,
    project_key: &str,
) -> Result<&'a IssueType> {
    match child_creation_mode {
        ChildCreationMode::StandardTask => issue_types
            .iter()
            .find(|issue_type| {
                issue_type.is_standard() && issue_type.name.eq_ignore_ascii_case("task")
            })
            .or_else(|| {
                issue_types
                    .iter()
                    .find(|issue_type| issue_type.is_standard())
            })
            .ok_or_else(|| eyre!("No standard issue types found for project {project_key}")),
        ChildCreationMode::Subtask => issue_types
            .iter()
            .find(|issue_type| issue_type.is_subtask())
            .ok_or_else(|| eyre!("No subtask type found for project {project_key}")),
    }
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
    let new_task_indices: Vec<usize> = tasks
        .iter()
        .enumerate()
        .filter(|(_, task)| task.key.is_none())
        .map(|(index, _)| index)
        .collect();

    let changed_task_indices: Vec<usize> = tasks
        .iter()
        .enumerate()
        .filter(|(_, task)| task.has_changes())
        .map(|(index, _)| index)
        .collect();

    if new_task_indices.is_empty() && changed_task_indices.is_empty() {
        return Err(eyre!("No new or changed tasks to import"));
    }

    let total_actions = new_task_indices.len() + changed_task_indices.len();
    let mut current_step = 0;

    // --- Update changed tasks ---
    for &task_index in &changed_task_indices {
        current_step += 1;
        let task = &tasks[task_index];
        let task_key = task.key.as_deref().unwrap();

        let _ = tx.send(Message::Progress(Progress {
            task_id: "import_tasks".into(),
            message: format!("Updating {task_key}: {}...", task.title),
            current: current_step,
            total: total_actions + 1,
        }));

        client
            .update_summary_and_description(task_key, &task.title, &task.description)
            .await?;

        tasks[task_index].imported_title = Some(tasks[task_index].title.clone());
        tasks[task_index].imported_description = Some(tasks[task_index].description.clone());
        write_tasks_json(tasks_path, &tasks)?;
    }

    // --- Create new tasks ---
    if new_task_indices.is_empty() {
        return Ok(());
    }

    let new_task_count = new_task_indices.len();

    // Single new task: update the current issue directly
    if new_task_count == 1 && changed_task_indices.is_empty() {
        let task_index = new_task_indices[0];
        let task = &tasks[task_index];

        let _ = tx.send(Message::Progress(Progress {
            task_id: "import_tasks".into(),
            message: format!("Updating {issue_key}..."),
            current: current_step + 1,
            total: total_actions + 1,
        }));

        client
            .append_description(issue_key, &task.description)
            .await?;

        tasks[task_index].key = Some(issue_key.to_string());
        tasks[task_index].imported_title = Some(tasks[task_index].title.clone());
        tasks[task_index].imported_description = Some(tasks[task_index].description.clone());
        write_tasks_json(tasks_path, &tasks)?;

        return Ok(());
    }

    let plan = plan_multi_task_import(issue_type_name);

    if plan.should_convert_parent_to_story {
        current_step += 1;
        let _ = tx.send(Message::Progress(Progress {
            task_id: "import_tasks".into(),
            message: format!("Converting {issue_key} to Story..."),
            current: current_step,
            total: total_actions + 1 + usize::from(plan.should_convert_parent_to_story),
        }));
        client.update_issue_type(issue_key, "Story").await?;
    }

    let issue_types = client.get_issue_types(project_key).await?;
    let child_issue_type =
        select_child_issue_type(&issue_types, plan.child_creation_mode, project_key)?;
    let child_issue_type_id = child_issue_type.id.clone();

    for &task_index in &new_task_indices {
        current_step += 1;
        let task = &tasks[task_index];
        let progress_label = match plan.child_creation_mode {
            ChildCreationMode::StandardTask => "Creating task",
            ChildCreationMode::Subtask => "Creating subtask",
        };

        let _ = tx.send(Message::Progress(Progress {
            task_id: "import_tasks".into(),
            message: format!("{progress_label}: {}...", task.title),
            current: current_step,
            total: total_actions + 1 + usize::from(plan.should_convert_parent_to_story),
        }));

        let created_key = client
            .create_issue(
                project_key,
                &child_issue_type_id,
                &task.title,
                Some(&task.description),
                Some(issue_key),
            )
            .await?;

        tasks[task_index].key = Some(created_key);
        tasks[task_index].imported_title = Some(tasks[task_index].title.clone());
        tasks[task_index].imported_description = Some(tasks[task_index].description.clone());
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
    fn task_entry_without_key_needs_action() {
        let task = TaskEntry {
            title: "New task".into(),
            description: "desc".into(),
            key: None,
            imported_title: None,
            imported_description: None,
        };
        assert!(task.needs_action());
        assert!(!task.has_changes());
    }

    #[test]
    fn task_entry_with_unchanged_content_does_not_need_action() {
        let task = TaskEntry {
            title: "Same title".into(),
            description: "Same desc".into(),
            key: Some("TEST-1".into()),
            imported_title: Some("Same title".into()),
            imported_description: Some("Same desc".into()),
        };
        assert!(!task.needs_action());
        assert!(!task.has_changes());
    }

    #[test]
    fn task_entry_with_changed_title_needs_action() {
        let task = TaskEntry {
            title: "Updated title".into(),
            description: "Same desc".into(),
            key: Some("TEST-1".into()),
            imported_title: Some("Old title".into()),
            imported_description: Some("Same desc".into()),
        };
        assert!(task.needs_action());
        assert!(task.has_changes());
    }

    #[test]
    fn task_entry_with_changed_description_needs_action() {
        let task = TaskEntry {
            title: "Same title".into(),
            description: "Updated desc".into(),
            key: Some("TEST-1".into()),
            imported_title: Some("Same title".into()),
            imported_description: Some("Old desc".into()),
        };
        assert!(task.needs_action());
        assert!(task.has_changes());
    }

    #[test]
    fn task_entry_without_imported_fields_does_not_detect_changes() {
        // Legacy tasks.json without imported_title/imported_description
        let task = TaskEntry {
            title: "Some title".into(),
            description: "Some desc".into(),
            key: Some("TEST-1".into()),
            imported_title: None,
            imported_description: None,
        };
        assert!(!task.needs_action());
        assert!(!task.has_changes());
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

    #[test]
    fn multi_task_import_creates_standard_tasks_under_epics() {
        let plan = plan_multi_task_import("Epic");

        assert_eq!(
            plan,
            MultiTaskImportPlan {
                child_creation_mode: ChildCreationMode::StandardTask,
                should_convert_parent_to_story: false,
            }
        );
    }

    #[test]
    fn multi_task_import_keeps_story_parents_as_subtask_parents() {
        let plan = plan_multi_task_import("Story");

        assert_eq!(
            plan,
            MultiTaskImportPlan {
                child_creation_mode: ChildCreationMode::Subtask,
                should_convert_parent_to_story: false,
            }
        );
    }

    #[test]
    fn multi_task_import_converts_non_story_non_epic_parents_before_subtasks() {
        let plan = plan_multi_task_import("Task");

        assert_eq!(
            plan,
            MultiTaskImportPlan {
                child_creation_mode: ChildCreationMode::Subtask,
                should_convert_parent_to_story: true,
            }
        );
    }

    #[test]
    fn standard_child_issue_type_prefers_task_issue_type() {
        let issue_types = vec![
            IssueType {
                id: "1".into(),
                name: "Story".into(),
                hierarchy_level: 0,
            },
            IssueType {
                id: "2".into(),
                name: "Task".into(),
                hierarchy_level: 0,
            },
            IssueType {
                id: "3".into(),
                name: "Sub-task".into(),
                hierarchy_level: -1,
            },
        ];

        let selected =
            select_child_issue_type(&issue_types, ChildCreationMode::StandardTask, "INI").unwrap();

        assert_eq!(selected.id, "2");
    }

    #[test]
    fn subtask_child_issue_type_requires_subtask_hierarchy() {
        let issue_types = vec![
            IssueType {
                id: "1".into(),
                name: "Task".into(),
                hierarchy_level: 0,
            },
            IssueType {
                id: "2".into(),
                name: "Sub-task".into(),
                hierarchy_level: -1,
            },
        ];

        let selected =
            select_child_issue_type(&issue_types, ChildCreationMode::Subtask, "INI").unwrap();

        assert_eq!(selected.id, "2");
    }
}
