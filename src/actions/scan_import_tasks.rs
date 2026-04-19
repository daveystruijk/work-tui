//! **Scan Import Tasks** — scans `$REPOS_DIR/opencode/changes/*/tasks.json`
//! for issue keys that have pending (un-imported) tasks.
//!
//! Returns a set of uppercased issue keys (e.g. `"INI-1234"`) extracted from
//! directory names that have at least one task entry without a `"key"` field.
//!
//! # Channel messages produced
//! - [`ActionMessage::PendingImportKeys`]

use std::collections::HashSet;
use std::path::Path;

use tokio::sync::mpsc;

use super::import_tasks::TaskEntry;
use super::ActionMessage;

/// Scan all tasks.json files and return issue keys with pending imports.
pub fn scan(repos_dir: &Path) -> HashSet<String> {
    let changes_dir = repos_dir.join("opencode").join("changes");
    let mut pending_keys = HashSet::new();

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

        let issue_key = extract_issue_key(dir_name);
        if issue_key.is_empty() {
            continue;
        }

        let tasks_path = path.join("tasks.json");
        let Ok(content) = std::fs::read_to_string(&tasks_path) else {
            continue;
        };

        let Ok(tasks) = serde_json::from_str::<Vec<TaskEntry>>(&content) else {
            continue;
        };

        let has_pending = tasks.iter().any(|task| task.key.is_none());
        if has_pending {
            pending_keys.insert(issue_key);
        }
    }

    pending_keys
}

/// Extract the issue key from a directory name like `ini-1234-some-description`.
/// Returns the uppercased key (e.g. `"INI-1234"`).
fn extract_issue_key(dir_name: &str) -> String {
    let mut parts = dir_name.splitn(3, '-');
    let Some(project) = parts.next() else {
        return String::new();
    };
    let Some(number) = parts.next() else {
        return String::new();
    };

    if number.chars().all(|c| c.is_ascii_digit()) && !number.is_empty() {
        format!("{}-{}", project.to_uppercase(), number)
    } else {
        String::new()
    }
}

/// Spawn the scan as a background task.
pub fn spawn(tx: mpsc::UnboundedSender<ActionMessage>, repos_dir: std::path::PathBuf) {
    super::spawn_action(tx, "Scanning import tasks", |tx| async move {
        let keys = tokio::task::spawn_blocking(move || scan(&repos_dir))
            .await
            .unwrap_or_default();
        let _ = tx.send(ActionMessage::PendingImportKeys(keys));
    });
}
