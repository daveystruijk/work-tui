//! **Sync Story Statuses** — automatically transitions parent stories based on
//! the statuses of their child tickets.
//!
//! After issues or story children are loaded, this action examines each story
//! that has loaded children and derives the appropriate status:
//!
//! - If **any** child is in "Review" → story should transition to "Review"
//! - If all non-backlog children are "In Progress" → story should transition to "In Progress"
//!
//! Stories that already have the correct status are skipped. When a story is
//! transitioned, it is also moved to the active board.
//!
//! # Channel messages produced
//! - [`Message::StoryStatusSynced`]

use std::collections::HashMap;

use color_eyre::Result;
use tokio::sync::mpsc;

use super::Message;
use crate::apis::jira::{Issue, JiraClient};

/// A single story that needs its status synced.
#[derive(Debug, Clone)]
pub struct StorySyncEntry {
    pub story_key: String,
    pub derived_status: DerivedStatus,
}

/// The status a story should have based on its children.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DerivedStatus {
    Review,
    InProgress,
}

impl DerivedStatus {
    pub fn label(&self) -> &'static str {
        match self {
            DerivedStatus::Review => "Review",
            DerivedStatus::InProgress => "In Progress",
        }
    }

    /// Keywords to match against available Jira transition names.
    fn transition_keyword(&self) -> &'static str {
        match self {
            DerivedStatus::Review => "review",
            DerivedStatus::InProgress => "progress",
        }
    }

    fn fallback_keyword(&self) -> Option<&'static str> {
        match self {
            DerivedStatus::InProgress => Some("start"),
            DerivedStatus::Review => None,
        }
    }
}

/// Derive the target status for a story based on its children's statuses.
/// Returns `None` if no status change is needed.
pub fn derive_story_status(story: &Issue, children: &[Issue]) -> Option<DerivedStatus> {
    if children.is_empty() {
        return None;
    }

    let story_status = story
        .status()
        .map(|s| s.name)
        .unwrap_or_default()
        .to_lowercase();

    let child_statuses: Vec<String> = children
        .iter()
        .filter_map(|child| child.status().map(|s| s.name.to_lowercase()))
        .collect();

    if child_statuses.is_empty() {
        return None;
    }

    // Skip stories that are already done or canceled
    if story_status.contains("done") || story_status.contains("cancel") {
        return None;
    }

    let has_review = child_statuses.iter().any(|s| s.contains("review"));

    let non_backlog_children: Vec<&String> = child_statuses
        .iter()
        .filter(|s| !s.contains("plan") && !s.contains("proposed") && !s.contains("backlog"))
        .collect();

    let all_in_progress = !non_backlog_children.is_empty()
        && non_backlog_children.iter().all(|s| s.contains("progress"));

    if has_review {
        if story_status.contains("review") {
            return None; // Already in review
        }
        return Some(DerivedStatus::Review);
    }

    if all_in_progress {
        if story_status.contains("progress") {
            return None; // Already in progress
        }
        return Some(DerivedStatus::InProgress);
    }

    None
}

/// Compute all stories that need status syncing from the current app state.
pub fn compute_sync_entries(
    issues: &[Issue],
    story_children: &HashMap<String, Vec<Issue>>,
) -> Vec<StorySyncEntry> {
    let mut entries = Vec::new();

    for (story_key, children) in story_children {
        let Some(story) = issues.iter().find(|i| i.key == *story_key) else {
            continue;
        };

        if !crate::issue::is_expandable(story) {
            continue;
        }

        let Some(derived_status) = derive_story_status(story, children) else {
            continue;
        };

        entries.push(StorySyncEntry {
            story_key: story_key.clone(),
            derived_status,
        });
    }

    entries
}

/// Spawn the sync action for a set of stories.
pub fn spawn(tx: mpsc::UnboundedSender<Message>, client: JiraClient, entries: Vec<StorySyncEntry>) {
    if entries.is_empty() {
        return;
    }

    super::spawn_action(
        tx,
        "sync_story_statuses",
        "Syncing story statuses",
        |tx| async move {
            let mut synced = Vec::new();
            for entry in entries {
                match sync_one(&client, &entry).await {
                    Ok(()) => synced.push((entry.story_key, entry.derived_status)),
                    Err(err) => {
                        tracing::warn!(
                            story = %entry.story_key,
                            target = entry.derived_status.label(),
                            "Failed to sync story status: {err}"
                        );
                    }
                }
            }
            let _ = tx.send(Message::StoryStatusSynced(synced));
        },
    );
}

async fn sync_one(client: &JiraClient, entry: &StorySyncEntry) -> Result<()> {
    let transitions = client.get_transitions(&entry.story_key).await?;

    let keyword = entry.derived_status.transition_keyword();
    let transition = transitions
        .iter()
        .find(|t| t.name.to_lowercase().contains(keyword))
        .or_else(|| {
            entry
                .derived_status
                .fallback_keyword()
                .and_then(|fallback| {
                    transitions
                        .iter()
                        .find(|t| t.name.to_lowercase().contains(fallback))
                })
        });

    let Some(transition) = transition else {
        return Ok(()); // No matching transition available, skip silently
    };

    client
        .transition_issue(&entry.story_key, &transition.id)
        .await?;

    // Best-effort board move — don't fail the whole sync if this errors
    let _ = client.move_issue_to_active_board(&entry.story_key).await;

    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::fixtures::test_issue;

    use super::*;

    fn issue_with_status(key: &str, status: &str, issue_type: &str) -> Issue {
        let mut issue = test_issue();
        issue.key = key.to_string();
        issue.fields.insert(
            "status".to_string(),
            json!({
                "description": "",
                "iconUrl": "",
                "id": "3",
                "name": status,
                "self": "http://localhost/status/3"
            }),
        );
        issue.fields.insert(
            "issuetype".to_string(),
            json!({
                "description": "",
                "iconUrl": "",
                "id": "10000",
                "name": issue_type,
                "self": "http://localhost/issuetype/10000",
                "subtask": false
            }),
        );
        issue
    }

    #[test]
    fn derives_review_when_any_child_in_review() {
        let story = issue_with_status("STORY-1", "In Progress", "Story");
        let children = vec![
            issue_with_status("TASK-1", "In Progress", "Task"),
            issue_with_status("TASK-2", "Review", "Task"),
        ];

        let result = derive_story_status(&story, &children);

        assert_eq!(result, Some(DerivedStatus::Review));
    }

    #[test]
    fn derives_in_progress_when_all_active_children_in_progress() {
        let story = issue_with_status("STORY-1", "Proposed", "Story");
        let children = vec![
            issue_with_status("TASK-1", "In Progress", "Task"),
            issue_with_status("TASK-2", "In Progress", "Task"),
        ];

        let result = derive_story_status(&story, &children);

        assert_eq!(result, Some(DerivedStatus::InProgress));
    }

    #[test]
    fn ignores_backlog_children_for_in_progress_derivation() {
        let story = issue_with_status("STORY-1", "Proposed", "Story");
        let children = vec![
            issue_with_status("TASK-1", "In Progress", "Task"),
            issue_with_status("TASK-2", "Proposed", "Task"),
        ];

        let result = derive_story_status(&story, &children);

        // TASK-2 is backlog (proposed), only TASK-1 is active and in progress
        assert_eq!(result, Some(DerivedStatus::InProgress));
    }

    #[test]
    fn no_change_when_story_already_in_review() {
        let story = issue_with_status("STORY-1", "Review", "Story");
        let children = vec![issue_with_status("TASK-1", "Review", "Task")];

        let result = derive_story_status(&story, &children);

        assert_eq!(result, None);
    }

    #[test]
    fn no_change_when_story_already_in_progress() {
        let story = issue_with_status("STORY-1", "In Progress", "Story");
        let children = vec![issue_with_status("TASK-1", "In Progress", "Task")];

        let result = derive_story_status(&story, &children);

        assert_eq!(result, None);
    }

    #[test]
    fn no_change_when_story_is_done() {
        let story = issue_with_status("STORY-1", "Done", "Story");
        let children = vec![issue_with_status("TASK-1", "Review", "Task")];

        let result = derive_story_status(&story, &children);

        assert_eq!(result, None);
    }

    #[test]
    fn no_change_with_mixed_statuses() {
        let story = issue_with_status("STORY-1", "Proposed", "Story");
        let children = vec![
            issue_with_status("TASK-1", "In Progress", "Task"),
            issue_with_status("TASK-2", "To Do", "Task"),
        ];

        let result = derive_story_status(&story, &children);

        // Mixed active statuses (not all in progress, none in review)
        assert_eq!(result, None);
    }

    #[test]
    fn no_change_with_empty_children() {
        let story = issue_with_status("STORY-1", "Proposed", "Story");

        let result = derive_story_status(&story, &[]);

        assert_eq!(result, None);
    }

    #[test]
    fn compute_sync_entries_finds_stories_needing_sync() {
        let story = issue_with_status("STORY-1", "Proposed", "Story");
        let task = issue_with_status("TASK-1", "In Progress", "Task");
        let issues = vec![story];
        let mut story_children = HashMap::new();
        story_children.insert("STORY-1".to_string(), vec![task]);

        let entries = compute_sync_entries(&issues, &story_children);

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].story_key, "STORY-1");
        assert_eq!(entries[0].derived_status, DerivedStatus::InProgress);
    }

    #[test]
    fn compute_sync_entries_skips_non_expandable() {
        let task = issue_with_status("TASK-1", "Proposed", "Task");
        let child = issue_with_status("TASK-2", "In Progress", "Task");
        let issues = vec![task];
        let mut story_children = HashMap::new();
        story_children.insert("TASK-1".to_string(), vec![child]);

        let entries = compute_sync_entries(&issues, &story_children);

        assert!(entries.is_empty());
    }
}
