//! **Scan Import Tasks** — scans openspec change tasks for pending imports
//! for issue keys that have pending (un-imported) tasks.
//!
//! Returns a set of uppercased issue keys (e.g. `"INI-1234"`) extracted from
//! directory names that have at least one task entry without a `"key"` field.
//!
//! # Channel messages produced
//! - [`Message::PendingImportKeys`]

use std::collections::HashSet;
use std::path::Path;

use tokio::sync::mpsc;

use super::import_tasks::{openspec_changes_dir, TaskEntry};
use super::Message;

/// Scan all tasks.json files and return issue keys with pending imports.
pub fn scan(repos_dir: &Path) -> HashSet<String> {
    let mut pending_keys = HashSet::new();

    let Some(changes_dir) = openspec_changes_dir(repos_dir) else {
        return pending_keys;
    };

    let Ok(entries) = std::fs::read_dir(&changes_dir) else {
        return pending_keys;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let Some(dir_name) = path.file_name().and_then(|os| os.to_str()) else {
            continue;
        };

        let Some(issue_key) = extract_issue_key(dir_name) else {
            continue;
        };

        let tasks_path = path.join("tasks.json");
        let Ok(content) = std::fs::read_to_string(&tasks_path) else {
            continue;
        };

        let Ok(tasks) = serde_json::from_str::<Vec<TaskEntry>>(&content) else {
            continue;
        };

        if tasks.iter().any(|task| task.key.is_none()) {
            pending_keys.insert(issue_key);
        }
    }

    pending_keys
}

/// Extract the issue key from a directory name like `ini-1234-some-description`.
/// Returns the uppercased key (e.g. `"INI-1234"`).
fn extract_issue_key(dir_name: &str) -> Option<String> {
    let mut parts = dir_name.splitn(3, '-');
    let Some(project) = parts.next() else {
        return None;
    };
    let Some(number) = parts.next() else {
        return None;
    };

    if number.chars().all(|c| c.is_ascii_digit()) && !number.is_empty() {
        return Some(format!("{}-{}", project.to_uppercase(), number));
    }

    None
}

/// Spawn the scan as a background task.
pub fn spawn(tx: mpsc::UnboundedSender<Message>, repos_dir: std::path::PathBuf) {
    super::spawn_action(tx, "Scanning import tasks", |tx| async move {
        let keys = tokio::task::spawn_blocking(move || scan(&repos_dir))
            .await
            .unwrap_or_default();
        let _ = tx.send(Message::PendingImportKeys(keys));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
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
    fn scans_pending_tasks_from_openspec_changes() {
        let repos_dir = TestReposDir::new("scan-openspec-pending-tasks");
        let tasks_path = repos_dir
            .path
            .join("openspec")
            .join("changes")
            .join("ini-4347-add-note")
            .join("tasks.json");

        fs::create_dir_all(tasks_path.parent().unwrap()).unwrap();
        fs::write(
            &tasks_path,
            r#"[
  {
    "title": "Add note",
    "description": "Update note formatting"
  }
]"#,
        )
        .unwrap();

        let ignored_tasks_path = repos_dir
            .path
            .join("opencode")
            .join("changes")
            .join("ini-9999-ignore-me")
            .join("tasks.json");

        fs::create_dir_all(ignored_tasks_path.parent().unwrap()).unwrap();
        fs::write(
            &ignored_tasks_path,
            r#"[
  {
    "title": "Ignore me",
    "description": "Should not count"
  }
]"#,
        )
        .unwrap();

        let pending_keys = scan(&repos_dir.path);

        assert_eq!(pending_keys, HashSet::from([String::from("INI-4347")]));
    }
}
