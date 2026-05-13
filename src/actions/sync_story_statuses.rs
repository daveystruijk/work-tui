//! **Sync Story Statuses** — automatically transitions parent stories based on
//! the statuses of their child tickets.
//!
//! After issues or story children are loaded, this action examines each story
//! that has loaded children and derives the appropriate status based on the
//! "highest watermark" of child statuses:
//!
//! - If **all** children are backlog (plan/proposed/backlog) → story should be "Planned"
//! - If **any** child is in progress → story should be "In Progress"
//! - If all non-backlog children are in review (or done) → story should be "Review"
//! - If **all** children are done → story should be "Done"
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
    Planned,
    InProgress,
    Review,
    Done,
}

impl DerivedStatus {
    pub fn label(&self) -> &'static str {
        match self {
            DerivedStatus::Planned => "Planned",
            DerivedStatus::InProgress => "In Progress",
            DerivedStatus::Review => "Review",
            DerivedStatus::Done => "Done",
        }
    }

    /// Keywords to match against available Jira transition names.
    fn transition_keyword(&self) -> &'static str {
        match self {
            DerivedStatus::Planned => "plan",
            DerivedStatus::InProgress => "progress",
            DerivedStatus::Review => "review",
            DerivedStatus::Done => "done",
        }
    }

    fn fallback_keyword(&self) -> Option<&'static str> {
        match self {
            DerivedStatus::Planned => Some("backlog"),
            DerivedStatus::InProgress => Some("start"),
            DerivedStatus::Review => None,
            DerivedStatus::Done => Some("close"),
        }
    }

    /// Whether the story's current status already matches this derived status.
    fn matches_current(&self, story_status: &str) -> bool {
        match self {
            DerivedStatus::Planned => {
                story_status.contains("plan")
                    || story_status.contains("proposed")
                    || story_status.contains("backlog")
            }
            DerivedStatus::InProgress => story_status.contains("progress"),
            DerivedStatus::Review => story_status.contains("review"),
            DerivedStatus::Done => story_status.contains("done"),
        }
    }
}

/// Classify a single status string into a tier.
fn classify_status(status: &str) -> StatusTier {
    if status.contains("done") || status.contains("cancel") || status.contains("closed") {
        StatusTier::Done
    } else if status.contains("review") {
        StatusTier::Review
    } else if status.contains("progress") {
        StatusTier::InProgress
    } else {
        // plan, proposed, backlog, to do, etc.
        StatusTier::Backlog
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum StatusTier {
    Backlog,
    InProgress,
    Review,
    Done,
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

    let tiers: Vec<StatusTier> = children
        .iter()
        .filter_map(|child| child.status().map(|s| classify_status(&s.name.to_lowercase())))
        .collect();

    if tiers.is_empty() {
        return None;
    }

    // Skip stories that are already canceled
    if story_status.contains("cancel") {
        return None;
    }

    let all_backlog = tiers.iter().all(|t| *t == StatusTier::Backlog);
    let all_done = tiers.iter().all(|t| *t == StatusTier::Done);
    let has_in_progress = tiers.iter().any(|t| *t == StatusTier::InProgress);
    let has_backlog = tiers.iter().any(|t| *t == StatusTier::Backlog);
    let all_review_or_done = tiers
        .iter()
        .all(|t| *t == StatusTier::Review || *t == StatusTier::Done);

    let derived = if all_done {
        DerivedStatus::Done
    } else if all_backlog {
        DerivedStatus::Planned
    } else if all_review_or_done {
        // Only review when every child is review or done (no backlog, no in-progress)
        DerivedStatus::Review
    } else if has_in_progress || has_backlog {
        // Any in-progress work, or a mix of review/backlog → In Progress
        DerivedStatus::InProgress
    } else {
        DerivedStatus::Review
    };

    if derived.matches_current(&story_status) {
        return None;
    }

    Some(derived)
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
    fn derives_in_progress_when_review_and_in_progress_mixed() {
        let story = issue_with_status("STORY-1", "Proposed", "Story");
        let children = vec![
            issue_with_status("TASK-1", "In Progress", "Task"),
            issue_with_status("TASK-2", "Review", "Task"),
        ];

        let result = derive_story_status(&story, &children);

        // In Progress work still exists, so story stays In Progress
        assert_eq!(result, Some(DerivedStatus::InProgress));
    }

    #[test]
    fn derives_review_when_all_active_children_in_review() {
        let story = issue_with_status("STORY-1", "In Progress", "Story");
        let children = vec![
            issue_with_status("TASK-1", "Review", "Task"),
            issue_with_status("TASK-2", "Review", "Task"),
        ];

        let result = derive_story_status(&story, &children);

        assert_eq!(result, Some(DerivedStatus::Review));
    }

    #[test]
    fn derives_in_progress_when_review_and_backlog_mixed() {
        let story = issue_with_status("STORY-1", "Proposed", "Story");
        let children = vec![
            issue_with_status("TASK-1", "Review", "Task"),
            issue_with_status("TASK-2", "Proposed", "Task"),
        ];

        let result = derive_story_status(&story, &children);

        // Backlog items remain, so story is In Progress (not Review)
        assert_eq!(result, Some(DerivedStatus::InProgress));
    }

    #[test]
    fn derives_review_when_mix_of_review_and_done() {
        let story = issue_with_status("STORY-1", "In Progress", "Story");
        let children = vec![
            issue_with_status("TASK-1", "Review", "Task"),
            issue_with_status("TASK-2", "Done", "Task"),
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
    fn derives_in_progress_with_backlog_and_in_progress_children() {
        let story = issue_with_status("STORY-1", "Proposed", "Story");
        let children = vec![
            issue_with_status("TASK-1", "In Progress", "Task"),
            issue_with_status("TASK-2", "Proposed", "Task"),
        ];

        let result = derive_story_status(&story, &children);

        assert_eq!(result, Some(DerivedStatus::InProgress));
    }

    #[test]
    fn derives_planned_when_all_children_are_backlog() {
        let story = issue_with_status("STORY-1", "In Progress", "Story");
        let children = vec![
            issue_with_status("TASK-1", "Proposed", "Task"),
            issue_with_status("TASK-2", "Backlog", "Task"),
        ];

        let result = derive_story_status(&story, &children);

        assert_eq!(result, Some(DerivedStatus::Planned));
    }

    #[test]
    fn derives_done_when_all_children_are_done() {
        let story = issue_with_status("STORY-1", "In Progress", "Story");
        let children = vec![
            issue_with_status("TASK-1", "Done", "Task"),
            issue_with_status("TASK-2", "Done", "Task"),
        ];

        let result = derive_story_status(&story, &children);

        assert_eq!(result, Some(DerivedStatus::Done));
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
    fn no_change_when_story_already_planned() {
        let story = issue_with_status("STORY-1", "Proposed", "Story");
        let children = vec![issue_with_status("TASK-1", "Proposed", "Task")];

        let result = derive_story_status(&story, &children);

        assert_eq!(result, None);
    }

    #[test]
    fn no_change_when_story_already_done() {
        let story = issue_with_status("STORY-1", "Done", "Story");
        let children = vec![issue_with_status("TASK-1", "Done", "Task")];

        let result = derive_story_status(&story, &children);

        assert_eq!(result, None);
    }

    #[test]
    fn no_change_when_story_is_canceled() {
        let story = issue_with_status("STORY-1", "Canceled", "Story");
        let children = vec![issue_with_status("TASK-1", "Review", "Task")];

        let result = derive_story_status(&story, &children);

        assert_eq!(result, None);
    }

    #[test]
    fn derives_in_progress_with_mixed_non_review_statuses() {
        let story = issue_with_status("STORY-1", "Proposed", "Story");
        let children = vec![
            issue_with_status("TASK-1", "In Progress", "Task"),
            issue_with_status("TASK-2", "To Do", "Task"),
        ];

        let result = derive_story_status(&story, &children);

        // "To Do" is backlog tier, "In Progress" is active → In Progress
        assert_eq!(result, Some(DerivedStatus::InProgress));
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
