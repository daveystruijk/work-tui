pub mod columns;
pub mod row;

use std::collections::{HashMap, HashSet};

use color_eyre::{eyre::eyre, Result};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nucleo_matcher::{
    pattern::{Atom, AtomKind, CaseMatching, Normalization},
    Config, Matcher, Utf32Str,
};
use ratatui::{
    layout::Constraint,
    style::{Modifier, Style},
    widgets::{Block, Cell, HighlightSpacing, Row, Table, TableState},
    Frame,
};

use crate::actions;
use crate::actions::Message;
use crate::apis::jira::Issue;
use crate::app::{AppView, DisplayRow, InlineNewView, InputFocus, ListSection};
use crate::theme::Theme;
use crate::ticket::{Ticket, TicketStore};
use crate::ui::confirm_dialog::{ConfirmAction, ConfirmDialogView};
use crate::ui::{ImportTasksView, LabelPickerView};
use tokio::process::Command;

use super::{
    adjust_scroll_offset, max_col_width, move_selected_index, CellMap, UiAnimationView, COLUMNS,
};

pub use row::ListRenderContext;

/// Returns true if the issue's status indicates it belongs in the backlog section.
fn is_backlog_status(issue: &Issue) -> bool {
    let name = issue
        .status()
        .map(|s| s.name)
        .unwrap_or_default()
        .to_lowercase();
    name.contains("plan") || name.contains("proposed")
}

/// Returns true if the parent has at least one child matching the given section filter.
/// For expandable children (stories/epics), recurses into their own children instead
/// of using the child's own status.
fn has_children_in_section(
    parent_key: &str,
    issues: &[Issue],
    parent_groups: &HashMap<String, (String, Vec<usize>)>,
    story_children: &HashMap<String, Vec<Issue>>,
    section: ListSection,
) -> bool {
    let child_in_section = |issue: &Issue| {
        let expandable = crate::issue::is_expandable(issue)
            || story_children.contains_key(&issue.key)
            || parent_groups.contains_key(&issue.key);
        if expandable {
            return has_children_in_section(
                &issue.key,
                issues,
                parent_groups,
                story_children,
                section,
            );
        }
        match section {
            ListSection::Board => !is_backlog_status(issue),
            ListSection::Backlog => is_backlog_status(issue),
        }
    };

    parent_groups
        .get(parent_key)
        .into_iter()
        .flat_map(|(_, children)| children.iter())
        .any(|idx| child_in_section(&issues[*idx]))
        || story_children
            .get(parent_key)
            .into_iter()
            .flat_map(|children| children.iter())
            .any(child_in_section)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use insta::assert_snapshot;
    use serde_json::json;

    use ratatui::style::{Modifier, Style};

    use crate::ticket::{TicketSources, TicketStore};
    use crate::ui::render;
    use crate::{
        apis::jira::Issue,
        app::{DisplayRow, ListSection},
        fixtures::{render_to_string, selected_issue_app, test_issue},
    };

    fn ticket_store_from(
        issues: &[Issue],
        story_children: &HashMap<String, Vec<Issue>>,
    ) -> TicketStore {
        TicketStore::from_sources(&TicketSources {
            issues,
            story_children,
            github_prs: &HashMap::new(),
            active_branches: &HashMap::new(),
            repo_entries: &[],
        })
    }

    use super::ListView;

    #[test]
    fn snapshots_list_view() {
        let mut app = selected_issue_app();
        let rendered = render_to_string(120, 16, |frame| {
            render(&mut app, frame);
        });

        assert_snapshot!("list_view", rendered);
    }

    #[test]
    fn snapshots_list_view_with_dev_active_issue() {
        use crate::fixtures::pr::test_pr;

        let mut app = selected_issue_app();
        let issue_key = app.issues[0].key.clone();
        app.github_prs.insert(issue_key.clone(), test_pr());
        let rendered = render_to_string(120, 16, |frame| {
            render(&mut app, frame);
        });

        assert_snapshot!("list_view_with_dev_active_issue", rendered);
    }

    #[test]
    fn board_story_starts_expanded_with_children() {
        let issues = vec![
            story_issue("TEST-1", "Story parent"),
            task_issue_with_parent("TASK-1", "Board task", "TEST-1", "Story parent"),
        ];
        let story_children = HashMap::new();
        let mut list = ListView::default();

        list.rebuild_display_rows(&issues, &story_children);
        // Board stories with children default to expanded
        assert!(!list
            .collapsed_stories
            .contains(&("TEST-1".to_string(), ListSection::Board)));
        assert!(matches!(
            list.display_rows.as_slice(),
            [
                DisplayRow::SectionHeader { section: ListSection::Board, .. },
                DisplayRow::Ticket { key, depth: 0 },
                DisplayRow::Ticket { key: child_key, depth: 1 },
            ] if key == "TEST-1" && child_key == "TASK-1"
        ));
    }

    #[test]
    fn backlog_story_with_known_children_starts_expanded() {
        let issues = vec![
            backlog_story_issue("TEST-1", "Backlog story"),
            backlog_task_issue_with_parent("TASK-1", "Backlog task", "TEST-1", "Backlog story"),
        ];
        let story_children = HashMap::new();
        let mut list = ListView::default();

        list.rebuild_display_rows(&issues, &story_children);
        // Story with known children in parent_groups starts expanded in BACKLOG.
        assert!(!list
            .collapsed_stories
            .contains(&("TEST-1".to_string(), ListSection::Backlog)));
        assert!(matches!(
            list.display_rows.as_slice(),
            [
                DisplayRow::SectionHeader { section: ListSection::Backlog, .. },
                DisplayRow::Ticket { key, depth: 0 },
                DisplayRow::Ticket { key: child_key, depth: 1 },
            ] if key == "TEST-1" && child_key == "TASK-1"
        ));
    }

    #[test]
    fn nested_story_renders_only_under_its_parent() {
        let issues = vec![
            story_issue("EPIC-1", "Epic parent"),
            story_issue_with_parent("STORY-1", "Child story", "EPIC-1", "Epic parent"),
            task_issue_with_parent("TASK-1", "Nested task", "STORY-1", "Child story"),
        ];
        let story_children = HashMap::new();
        let mut list = ListView::default();

        list.rebuild_display_rows(&issues, &story_children);
        list.selected_index = 1;
        list.expand_story(
            &ticket_store_from(&issues, &story_children),
            &issues,
            &story_children,
        );
        list.selected_index = 2;
        list.expand_story(
            &ticket_store_from(&issues, &story_children),
            &issues,
            &story_children,
        );

        // STORY-1 appears once in BOARD (expanded under EPIC-1); BACKLOG epic starts collapsed
        assert_eq!(story_header_count(&list.display_rows, "STORY-1"), 1);
        // Verify BOARD section has the correct nested structure
        assert!(matches!(
            &list.display_rows[..4],
            [
                DisplayRow::SectionHeader { section: ListSection::Board, .. },
                DisplayRow::Ticket { key: epic_key, depth: 0 },
                DisplayRow::Ticket { key: story_key, depth: 1 },
                DisplayRow::Ticket { key: task_key, depth: 2 },
            ] if epic_key == "EPIC-1" && story_key == "STORY-1" && task_key == "TASK-1"
        ));
    }

    #[test]
    fn child_story_is_not_duplicated_when_loaded_twice() {
        let issues = vec![
            story_issue_with_parent("STORY-1", "Child story", "EPIC-1", "Epic parent"),
            task_issue_with_parent("TASK-1", "Nested task", "STORY-1", "Child story"),
        ];
        let story_children = HashMap::from([(
            "EPIC-1".to_string(),
            vec![story_issue_with_parent(
                "STORY-1",
                "Child story",
                "EPIC-1",
                "Epic parent",
            )],
        )]);
        let mut list = ListView::default();

        list.rebuild_display_rows(&issues, &story_children);
        list.selected_index = 1;
        list.expand_story(
            &ticket_store_from(&issues, &story_children),
            &issues,
            &story_children,
        );

        assert_eq!(story_header_count(&list.display_rows, "STORY-1"), 1);
    }

    fn story_issue(key: &str, summary: &str) -> Issue {
        issue_with_parent(key, summary, "Story", None)
    }

    fn backlog_story_issue(key: &str, summary: &str) -> Issue {
        let mut issue = story_issue(key, summary);
        issue.fields.insert(
            "status".to_string(),
            json!({
                "description": "",
                "iconUrl": "",
                "id": "1",
                "name": "Proposed",
                "self": "http://localhost/status/1"
            }),
        );
        issue
    }

    fn story_issue_with_parent(
        key: &str,
        summary: &str,
        parent_key: &str,
        parent_summary: &str,
    ) -> Issue {
        issue_with_parent(key, summary, "Story", Some((parent_key, parent_summary)))
    }

    fn task_issue_with_parent(
        key: &str,
        summary: &str,
        parent_key: &str,
        parent_summary: &str,
    ) -> Issue {
        issue_with_parent(key, summary, "Task", Some((parent_key, parent_summary)))
    }

    fn issue_with_parent(
        key: &str,
        summary: &str,
        issue_type: &str,
        parent: Option<(&str, &str)>,
    ) -> Issue {
        let mut issue = test_issue();
        issue.key = key.to_string();
        issue
            .fields
            .insert("summary".to_string(), json!(summary.to_string()));
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
        if let Some((parent_key, parent_summary)) = parent {
            issue.fields.insert(
                "parent".to_string(),
                json!({
                    "id": format!("{parent_key}-id"),
                    "key": parent_key,
                    "self": format!("http://localhost/rest/api/2/issue/{parent_key}"),
                    "fields": {
                        "summary": parent_summary,
                        "issuetype": {
                            "description": "",
                            "iconUrl": "",
                            "id": "10000",
                            "name": "Story",
                            "self": "http://localhost/issuetype/10000",
                            "subtask": false
                        }
                    }
                }),
            );
        }
        issue
    }

    fn epic_issue(key: &str, summary: &str) -> Issue {
        issue_with_parent(key, summary, "Epic", None)
    }

    fn backlog_epic_issue(key: &str, summary: &str) -> Issue {
        set_backlog_status(epic_issue(key, summary))
    }

    fn backlog_task_issue_with_parent(
        key: &str,
        summary: &str,
        parent_key: &str,
        parent_summary: &str,
    ) -> Issue {
        set_backlog_status(task_issue_with_parent(
            key,
            summary,
            parent_key,
            parent_summary,
        ))
    }

    fn set_backlog_status(mut issue: Issue) -> Issue {
        issue.fields.insert(
            "status".to_string(),
            json!({
                "description": "",
                "iconUrl": "",
                "id": "1",
                "name": "Proposed",
                "self": "http://localhost/status/1"
            }),
        );
        issue
    }

    fn format_display_rows(
        rows: &[DisplayRow],
        issues: &[Issue],
        story_children: &HashMap<String, Vec<Issue>>,
    ) -> String {
        let store = ticket_store_from(issues, story_children);
        let next_row_depth = |i: usize| -> Option<u8> {
            rows.get(i + 1).and_then(|r| match r {
                DisplayRow::Ticket { depth, .. }
                | DisplayRow::Loading { depth }
                | DisplayRow::Empty { depth } => Some(*depth),
                _ => None,
            })
        };

        let mut lines = Vec::new();
        let mut current_section = None;
        for (i, row) in rows.iter().enumerate() {
            match row {
                DisplayRow::SectionHeader { section, count } => {
                    current_section = Some(*section);
                    let label = match section {
                        ListSection::Board => "BOARD",
                        ListSection::Backlog => "BACKLOG",
                    };
                    lines.push(format!("── {label} ({count}) ──"));
                }
                DisplayRow::Ticket { key, depth } => {
                    let indent = "  ".repeat(*depth as usize);
                    let issue = store.get(key).map(|t| &t.issue);
                    let is_group_header = next_row_depth(i).map(|d| d > *depth).unwrap_or(false);
                    if is_group_header {
                        let summary = issue
                            .map(|i| i.summary().unwrap_or_default())
                            .unwrap_or_default();
                        let section_label = match current_section {
                            Some(ListSection::Board) => "board",
                            Some(ListSection::Backlog) => "backlog",
                            None => "none",
                        };
                        lines.push(format!("{indent}▸ {key} {summary} [{section_label}]"));
                    } else {
                        let issue = issue.unwrap_or_else(|| panic!("missing issue for key {key}"));
                        let summary = issue.summary().unwrap_or_default();
                        let status = issue.status().map(|s| s.name).unwrap_or_default();
                        lines.push(format!("{indent}• {key} {summary} [{status}]"));
                    }
                }
                DisplayRow::Loading { depth } => {
                    let indent = "  ".repeat(*depth as usize);
                    lines.push(format!("{indent}⟳ loading..."));
                }
                DisplayRow::Empty { depth } => {
                    let indent = "  ".repeat(*depth as usize);
                    lines.push(format!("{indent}(empty)"));
                }
                DisplayRow::InlineNew { depth } => {
                    let indent = "  ".repeat(*depth as usize);
                    lines.push(format!("{indent}+ new issue"));
                }
            }
        }
        lines.join("\n")
    }

    // ── Snapshot tests for complex listings ──

    /// Epic with board-status children: appears only in BOARD.
    #[test]
    fn snapshots_epic_with_board_children() {
        let issues = vec![
            epic_issue("EPIC-1", "Platform migration"),
            story_issue_with_parent(
                "STORY-1",
                "Migrate auth service",
                "EPIC-1",
                "Platform migration",
            ),
            task_issue_with_parent(
                "TASK-1",
                "Update OAuth config",
                "STORY-1",
                "Migrate auth service",
            ),
            task_issue_with_parent(
                "TASK-2",
                "Write integration tests",
                "STORY-1",
                "Migrate auth service",
            ),
        ];
        let story_children = HashMap::new();
        let mut list = ListView::default();

        list.rebuild_display_rows(&issues, &story_children);
        // Expand the epic and story in BOARD
        list.selected_index = 1;
        list.expand_story(
            &ticket_store_from(&issues, &story_children),
            &issues,
            &story_children,
        );
        list.selected_index = 2;
        list.expand_story(
            &ticket_store_from(&issues, &story_children),
            &issues,
            &story_children,
        );

        assert_snapshot!(
            "epic_with_board_children",
            format_display_rows(&list.display_rows, &issues, &story_children)
        );
    }

    /// Epic with only backlog children: appears only in BACKLOG, not BOARD.
    #[test]
    fn snapshots_epic_with_only_backlog_children() {
        let issues = vec![
            epic_issue("EPIC-1", "Future work"),
            backlog_task_issue_with_parent("TASK-1", "Research caching", "EPIC-1", "Future work"),
            backlog_task_issue_with_parent("TASK-2", "Draft RFC", "EPIC-1", "Future work"),
        ];
        let story_children = HashMap::new();
        let mut list = ListView::default();

        list.rebuild_display_rows(&issues, &story_children);

        assert_snapshot!(
            "epic_with_only_backlog_children",
            format_display_rows(&list.display_rows, &issues, &story_children)
        );
    }

    /// Epic with children in both sections: appears in both BOARD and BACKLOG.
    #[test]
    fn snapshots_epic_with_mixed_children() {
        let issues = vec![
            epic_issue("EPIC-1", "Cross-team initiative"),
            task_issue_with_parent(
                "TASK-1",
                "Implement API endpoint",
                "EPIC-1",
                "Cross-team initiative",
            ),
            task_issue_with_parent(
                "TASK-2",
                "Add monitoring",
                "EPIC-1",
                "Cross-team initiative",
            ),
            backlog_task_issue_with_parent(
                "TASK-3",
                "Plan rollout",
                "EPIC-1",
                "Cross-team initiative",
            ),
            backlog_task_issue_with_parent(
                "TASK-4",
                "Write docs",
                "EPIC-1",
                "Cross-team initiative",
            ),
        ];
        let story_children = HashMap::new();
        let mut list = ListView::default();

        list.rebuild_display_rows(&issues, &story_children);
        // Expand the epic in BOARD
        list.selected_index = 1;
        list.expand_story(
            &ticket_store_from(&issues, &story_children),
            &issues,
            &story_children,
        );

        assert_snapshot!(
            "epic_with_mixed_children",
            format_display_rows(&list.display_rows, &issues, &story_children)
        );
    }

    /// Story with no loaded children: appears in both sections (children unknown).
    #[test]
    fn snapshots_story_with_unknown_children() {
        let issues = vec![
            story_issue("STORY-1", "User authentication"),
            task_issue_with_parent("TASK-1", "Standalone board task", "PROJ-99", "Other parent"),
        ];
        let story_children = HashMap::new();
        let mut list = ListView::default();

        list.rebuild_display_rows(&issues, &story_children);

        assert_snapshot!(
            "story_with_unknown_children",
            format_display_rows(&list.display_rows, &issues, &story_children)
        );
    }

    /// Mixed listing: multiple epics and standalone tasks across both sections.
    #[test]
    fn snapshots_mixed_board_and_backlog() {
        let issues = vec![
            // Board epic with board children
            epic_issue("EPIC-1", "Active sprint work"),
            task_issue_with_parent("TASK-1", "Fix login bug", "EPIC-1", "Active sprint work"),
            task_issue_with_parent("TASK-2", "Update dashboard", "EPIC-1", "Active sprint work"),
            // Backlog epic with backlog children
            backlog_epic_issue("EPIC-2", "Next quarter planning"),
            backlog_task_issue_with_parent(
                "TASK-3",
                "Research competitors",
                "EPIC-2",
                "Next quarter planning",
            ),
            // Standalone board task
            issue_with_parent("TASK-4", "Hotfix deploy script", "Task", None),
            // Standalone backlog task
            set_backlog_status(issue_with_parent(
                "TASK-5",
                "Evaluate new framework",
                "Task",
                None,
            )),
        ];
        let story_children = HashMap::new();
        let mut list = ListView::default();

        list.rebuild_display_rows(&issues, &story_children);
        // Expand EPIC-1 in BOARD
        list.selected_index = 1;
        list.expand_story(
            &ticket_store_from(&issues, &story_children),
            &issues,
            &story_children,
        );

        assert_snapshot!(
            "mixed_board_and_backlog",
            format_display_rows(&list.display_rows, &issues, &story_children)
        );
    }

    /// Backlog epic with loaded children via story_children (fetched).
    #[test]
    fn snapshots_backlog_epic_with_fetched_children() {
        let issues = vec![backlog_epic_issue("EPIC-1", "Backlog epic")];
        let story_children = HashMap::from([(
            "EPIC-1".to_string(),
            vec![
                set_backlog_status(task_issue_with_parent(
                    "TASK-1",
                    "Backlog child 1",
                    "EPIC-1",
                    "Backlog epic",
                )),
                set_backlog_status(task_issue_with_parent(
                    "TASK-2",
                    "Backlog child 2",
                    "EPIC-1",
                    "Backlog epic",
                )),
            ],
        )]);
        let mut list = ListView::default();

        list.rebuild_display_rows(&issues, &story_children);

        assert_snapshot!(
            "backlog_epic_with_fetched_children",
            format_display_rows(&list.display_rows, &issues, &story_children)
        );
    }

    /// Epic with a nested story whose children are all backlog: story should
    /// not appear in BOARD, only in BACKLOG under the epic.
    #[test]
    fn snapshots_nested_story_with_only_backlog_children() {
        let issues = vec![
            epic_issue("EPIC-1", "Platform work"),
            story_issue_with_parent("STORY-1", "Auth migration", "EPIC-1", "Platform work"),
            backlog_task_issue_with_parent(
                "TASK-1",
                "Research OAuth providers",
                "STORY-1",
                "Auth migration",
            ),
            backlog_task_issue_with_parent(
                "TASK-2",
                "Draft migration plan",
                "STORY-1",
                "Auth migration",
            ),
            task_issue_with_parent("TASK-3", "Update CI pipeline", "EPIC-1", "Platform work"),
        ];
        let story_children = HashMap::new();
        let mut list = ListView::default();

        list.rebuild_display_rows(&issues, &story_children);

        assert_snapshot!(
            "nested_story_with_only_backlog_children",
            format_display_rows(&list.display_rows, &issues, &story_children)
        );
    }

    /// Story under epic with only backlog children — story appears in issues
    /// list, story_children, and has children via parent_groups. Should not
    /// appear in BOARD.
    #[test]
    fn snapshots_fetched_story_without_board_children() {
        let issues = vec![
            epic_issue("EPIC-1", "Platform work"),
            story_issue_with_parent("STORY-1", "Auth migration", "EPIC-1", "Platform work"),
            task_issue_with_parent("TASK-1", "Board task", "EPIC-1", "Platform work"),
            backlog_task_issue_with_parent(
                "TASK-2",
                "Backlog subtask",
                "STORY-1",
                "Auth migration",
            ),
            backlog_task_issue_with_parent(
                "TASK-3",
                "Another backlog subtask",
                "STORY-1",
                "Auth migration",
            ),
        ];
        let story_children = HashMap::from([(
            "EPIC-1".to_string(),
            vec![story_issue_with_parent(
                "STORY-1",
                "Auth migration",
                "EPIC-1",
                "Platform work",
            )],
        )]);
        let mut list = ListView::default();

        list.rebuild_display_rows(&issues, &story_children);

        assert_snapshot!(
            "fetched_story_without_board_children",
            format_display_rows(&list.display_rows, &issues, &story_children)
        );
    }

    /// Substring search matches contiguous characters case-insensitively.
    #[test]
    fn snapshots_search_substring_matches() {
        let issues = vec![
            epic_issue("EPIC-1", "Platform migration"),
            task_issue_with_parent("TASK-1", "Update config", "EPIC-1", "Platform migration"),
            task_issue_with_parent("TASK-2", "Write tests", "EPIC-1", "Platform migration"),
            story_issue("STORY-1", "Auth migration"),
            task_issue_with_parent("TASK-3", "Update OAuth", "STORY-1", "Auth migration"),
        ];
        let story_children = HashMap::new();
        let mut list = ListView::default();
        // "platf" substring-matches "Platform" (contiguous, case-insensitive)
        list.search_filter = "platf".to_string();

        list.rebuild_display_rows(&issues, &story_children);

        assert_snapshot!(
            "search_substring_matches",
            format_display_rows(&list.display_rows, &issues, &story_children)
        );
    }

    /// Non-contiguous characters do NOT match with substring strategy.
    #[test]
    fn search_substring_rejects_non_contiguous() {
        let issues = vec![epic_issue("EPIC-1", "Platform migration")];
        let story_children = HashMap::new();
        let mut list = ListView::default();
        // "pltfrm" has gaps — should NOT match with substring
        list.search_filter = "pltfrm".to_string();

        list.rebuild_display_rows(&issues, &story_children);

        let issue_rows: Vec<_> = list
            .display_rows
            .iter()
            .filter(|r| matches!(r, DisplayRow::Ticket { .. }))
            .collect();
        assert!(
            issue_rows.is_empty(),
            "non-contiguous query should not match any issues"
        );
    }

    /// Searching for an epic by name surfaces it even without matching children.
    #[test]
    fn snapshots_search_matches_epic() {
        let issues = vec![
            epic_issue("EPIC-1", "Platform migration"),
            task_issue_with_parent("TASK-1", "Update config", "EPIC-1", "Platform migration"),
            task_issue_with_parent("TASK-2", "Write tests", "EPIC-1", "Platform migration"),
        ];
        let story_children = HashMap::new();
        let mut list = ListView::default();
        list.search_filter = "platform".to_string();

        list.rebuild_display_rows(&issues, &story_children);

        assert_snapshot!(
            "search_matches_epic",
            format_display_rows(&list.display_rows, &issues, &story_children)
        );
    }

    /// Searching for a story by name surfaces it even without matching children.
    #[test]
    fn snapshots_search_matches_story() {
        let issues = vec![
            story_issue("STORY-1", "Auth migration"),
            task_issue_with_parent("TASK-1", "Update OAuth", "STORY-1", "Auth migration"),
            task_issue_with_parent("TASK-2", "Add tests", "STORY-1", "Auth migration"),
        ];
        let story_children = HashMap::new();
        let mut list = ListView::default();
        list.search_filter = "auth".to_string();

        list.rebuild_display_rows(&issues, &story_children);

        assert_snapshot!(
            "search_matches_story",
            format_display_rows(&list.display_rows, &issues, &story_children)
        );
    }

    /// Searching for a child still surfaces it under its parent group.
    #[test]
    fn snapshots_search_matches_child_under_story() {
        let issues = vec![
            story_issue("STORY-1", "Auth migration"),
            task_issue_with_parent("TASK-1", "Update OAuth", "STORY-1", "Auth migration"),
            task_issue_with_parent("TASK-2", "Add tests", "STORY-1", "Auth migration"),
        ];
        let story_children = HashMap::new();
        let mut list = ListView::default();
        list.search_filter = "oauth".to_string();

        list.rebuild_display_rows(&issues, &story_children);

        assert_snapshot!(
            "search_matches_child_under_story",
            format_display_rows(&list.display_rows, &issues, &story_children)
        );
    }

    /// Render-level test: searching highlights matched characters in the list.
    #[test]
    fn snapshots_search_highlight_render() {
        let mut app = selected_issue_app();
        // "snap" fuzzy-matches "Snapshot" in the issue summary
        app.list.search_filter = "snap".to_string();
        app.list
            .rebuild_display_rows(&app.issues.clone(), &app.story_children.clone());
        let rendered = render_to_string(120, 6, |frame| {
            render(&mut app, frame);
        });

        assert_snapshot!("search_highlight_render", rendered);
    }

    /// Unit test: highlight_spans splits text correctly at matched indices.
    #[test]
    fn highlight_spans_splits_at_match_indices() {
        let base = Style::default().fg(crate::theme::Theme::Text);
        let highlight = Style::default()
            .fg(crate::theme::Theme::SearchMatch)
            .add_modifier(Modifier::BOLD);

        // "Platform" with indices 0,4,5 matching "P", "f", "o"
        let spans = super::columns::highlight_spans("Platform", &[0, 4, 5], base, highlight);
        let texts: Vec<(&str, Style)> = spans
            .iter()
            .map(|s| (s.content.as_ref(), s.style))
            .collect();

        assert_eq!(
            texts,
            vec![
                ("P", highlight),
                ("lat", base),
                ("fo", highlight),
                ("rm", base),
            ]
        );
    }

    /// Unit test: highlight_spans returns single span when no indices.
    #[test]
    fn highlight_spans_no_matches() {
        let base = Style::default().fg(crate::theme::Theme::Text);
        let highlight = Style::default().fg(crate::theme::Theme::SearchMatch);

        let spans = super::columns::highlight_spans("Hello", &[], base, highlight);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content.as_ref(), "Hello");
        assert_eq!(spans[0].style, base);
    }

    fn story_header_count(rows: &[DisplayRow], key: &str) -> usize {
        rows.iter()
            .filter(|row| matches!(row, DisplayRow::Ticket { key: row_key, .. } if row_key == key))
            .count()
    }
}

#[derive(Debug, Clone, Default)]
pub struct ListView {
    pub area_height: u16,
    pub scroll_offset: usize,
    pub loading_children: HashSet<String>,
    pub selected_index: usize,
    pub display_rows: Vec<DisplayRow>,
    pub search_filter: String,
    pub collapsed_stories: HashSet<(String, ListSection)>,
    pub inline_new: Option<InlineNewView>,
    pub pending_import_keys: HashSet<String>,
}

impl ListView {
    fn has_ticket_row(&self, key: &str) -> bool {
        self.display_rows
            .iter()
            .any(|row| matches!(row, DisplayRow::Ticket { key: row_key, .. } if row_key == key))
    }

    pub fn handle_message(&mut self, msg: &Message) {
        match msg {
            Message::Issues(Ok(_)) => {
                self.loading_children.clear();
            }
            Message::ChildrenLoaded(parent_key, _) => {
                self.loading_children.remove(parent_key);
            }
            Message::PendingImportKeys(keys) => {
                self.pending_import_keys = keys.clone();
            }
            _ => {}
        }
    }

    pub fn start_loading_children(&mut self, parent_key: &str) {
        self.loading_children.insert(parent_key.to_string());
    }

    /// Returns the ticket for the currently selected display row, if any.
    pub fn selected_ticket<'a>(&self, ticket_store: &'a TicketStore) -> Option<&'a Ticket> {
        self.ticket_for_row(self.selected_index, ticket_store)
    }

    /// Returns the ticket for a given display row index, if any.
    pub fn ticket_for_row<'a>(
        &self,
        row_index: usize,
        ticket_store: &'a TicketStore,
    ) -> Option<&'a Ticket> {
        let row = self.display_rows.get(row_index)?;
        self.ticket_for_display_row(row, ticket_store)
    }

    /// Returns the ticket for a given display row, if any.
    pub fn ticket_for_display_row<'a>(
        &self,
        row: &DisplayRow,
        ticket_store: &'a TicketStore,
    ) -> Option<&'a Ticket> {
        match row {
            DisplayRow::Ticket { key, .. } => ticket_store.get(key),
            DisplayRow::InlineNew { .. }
            | DisplayRow::SectionHeader { .. }
            | DisplayRow::Loading { .. }
            | DisplayRow::Empty { .. } => None,
        }
    }

    /// Toggle collapse state for the story at the current selection.
    /// Returns the key if expansion needs a children fetch, None otherwise.
    pub fn toggle_story_collapse(
        &mut self,
        ticket_store: &TicketStore,
        issues: &[Issue],
        story_children: &HashMap<String, Vec<Issue>>,
    ) -> Option<String> {
        let key = match self.display_rows.get(self.selected_index) {
            Some(DisplayRow::Ticket { key, .. }) => key.clone(),
            _ => return None,
        };
        let ticket = ticket_store.get(&key)?;
        if !crate::issue::is_expandable(&ticket.issue) {
            return None;
        }
        let Some(section) = self.section_for_row(self.selected_index) else {
            return None;
        };
        let needs_fetch = if self.collapsed_stories.remove(&(key.clone(), section)) {
            self.maybe_needs_fetch(&key, story_children)
        } else {
            self.collapsed_stories.insert((key.clone(), section));
            None
        };
        self.rebuild_display_rows(issues, story_children);
        needs_fetch
    }

    /// Expand the story at the current selection.
    /// Returns the key if expansion needs a children fetch, None otherwise.
    pub fn expand_story(
        &mut self,
        ticket_store: &TicketStore,
        issues: &[Issue],
        story_children: &HashMap<String, Vec<Issue>>,
    ) -> Option<String> {
        let key = match self.display_rows.get(self.selected_index) {
            Some(DisplayRow::Ticket { key, .. }) => key.clone(),
            _ => return None,
        };
        let ticket = ticket_store.get(&key)?;
        if !crate::issue::is_expandable(&ticket.issue) {
            return None;
        }
        let Some(section) = self.section_for_row(self.selected_index) else {
            return None;
        };
        if !self.collapsed_stories.remove(&(key.clone(), section)) {
            return None;
        }
        let needs_fetch = self.maybe_needs_fetch(&key, story_children);
        self.rebuild_display_rows(issues, story_children);
        needs_fetch
    }

    /// Collapse the story at the current selection.
    pub fn collapse_story(
        &mut self,
        ticket_store: &TicketStore,
        issues: &[Issue],
        story_children: &HashMap<String, Vec<Issue>>,
    ) -> bool {
        let key = match self.display_rows.get(self.selected_index) {
            Some(DisplayRow::Ticket { key, .. }) => key.clone(),
            _ => return false,
        };
        let Some(ticket) = ticket_store.get(&key) else {
            return false;
        };
        if !crate::issue::is_expandable(&ticket.issue) {
            return false;
        }
        let Some(section) = self.section_for_row(self.selected_index) else {
            return false;
        };
        if self.collapsed_stories.contains(&(key.clone(), section)) {
            return false;
        }
        self.collapsed_stories.insert((key, section));
        self.rebuild_display_rows(issues, story_children);
        true
    }

    fn maybe_needs_fetch(
        &self,
        key: &str,
        story_children: &HashMap<String, Vec<Issue>>,
    ) -> Option<String> {
        if self.loading_children.contains(key) || story_children.contains_key(key) {
            return None;
        }
        Some(key.to_string())
    }

    /// Derive the section a row belongs to from the nearest preceding SectionHeader.
    fn section_for_row(&self, row_index: usize) -> Option<ListSection> {
        self.display_rows[..=row_index]
            .iter()
            .rev()
            .find_map(|row| match row {
                DisplayRow::SectionHeader { section, .. } => Some(*section),
                _ => None,
            })
    }

    /// Returns the story key and depth if the current selection is inside a
    /// story group.
    fn enclosing_story_key_and_depth(&self) -> Option<(String, u8)> {
        let current_depth = match &self.display_rows.get(self.selected_index)? {
            DisplayRow::Ticket { depth, .. }
            | DisplayRow::InlineNew { depth }
            | DisplayRow::Loading { depth }
            | DisplayRow::Empty { depth } => *depth,
            DisplayRow::SectionHeader { .. } => return None,
        };
        if current_depth == 0 {
            return None;
        }
        for i in (0..self.selected_index).rev() {
            match &self.display_rows[i] {
                DisplayRow::Ticket { key, depth, .. } if *depth < current_depth => {
                    return Some((key.clone(), *depth));
                }
                DisplayRow::Ticket { depth: 0, .. } => return None,
                _ => continue,
            }
        }
        None
    }

    /// Returns the story key and its depth for inline creation.
    /// Only returns a parent when the selected row is a group header
    /// (i.e. has children at a deeper depth below it), or when the
    /// selection is inside an existing story group.
    fn selected_story_or_enclosing(&self) -> Option<(String, u8)> {
        if let Some(DisplayRow::Ticket { key, depth, .. }) =
            self.display_rows.get(self.selected_index)
        {
            let is_group_header = self
                .display_rows
                .get(self.selected_index + 1)
                .map(|next| match next {
                    DisplayRow::Ticket { depth: d, .. }
                    | DisplayRow::Loading { depth: d }
                    | DisplayRow::Empty { depth: d } => *d > *depth,
                    _ => false,
                })
                .unwrap_or(false);
            if is_group_header {
                return Some((key.clone(), *depth));
            }
            return self.enclosing_story_key_and_depth();
        }
        if matches!(
            self.display_rows.get(self.selected_index),
            Some(DisplayRow::SectionHeader { .. })
        ) {
            return None;
        }
        self.enclosing_story_key_and_depth()
    }

    /// Start an inline new-issue row.
    pub fn start_inline_new(&mut self, project_key: String) -> bool {
        let story_key = self.selected_story_or_enclosing();

        let (insert_at, depth, parent_key) = if let Some((parent, story_depth)) = story_key {
            let child_depth = story_depth + 1;
            let group_end = self.find_story_group_end(self.selected_index);
            let replace_empty = matches!(
                self.display_rows.get(group_end),
                Some(DisplayRow::Empty { .. })
            );
            if replace_empty {
                (group_end, child_depth, Some(parent))
            } else {
                (group_end + 1, child_depth, Some(parent))
            }
        } else {
            let at = self.selected_index + 1;
            (at, 0u8, None)
        };

        if matches!(
            self.display_rows.get(insert_at),
            Some(DisplayRow::Empty { .. })
        ) {
            self.display_rows[insert_at] = DisplayRow::InlineNew { depth };
        } else {
            self.display_rows
                .insert(insert_at, DisplayRow::InlineNew { depth });
        }

        let state = InlineNewView {
            summary: String::new(),
            parent_key,
            project_key,
            row_index: insert_at,
        };
        self.inline_new = Some(state);
        self.selected_index = insert_at;
        true
    }

    /// Cancel the inline new issue and remove the placeholder row.
    pub fn cancel_inline_new(&mut self) {
        let Some(state) = self.inline_new.take() else {
            return;
        };
        self.remove_inline_row(state.row_index);
    }

    /// Find the last row index belonging to the story group that contains `from`.
    fn find_story_group_end(&self, from: usize) -> usize {
        let base_depth = match &self.display_rows[from] {
            DisplayRow::Ticket { depth, .. }
            | DisplayRow::InlineNew { depth }
            | DisplayRow::Loading { depth }
            | DisplayRow::Empty { depth } => *depth,
            DisplayRow::SectionHeader { .. } => return from,
        };
        let mut end = from;
        for i in (from + 1)..self.display_rows.len() {
            let row_depth = match &self.display_rows[i] {
                DisplayRow::Ticket { depth, .. }
                | DisplayRow::InlineNew { depth }
                | DisplayRow::Loading { depth }
                | DisplayRow::Empty { depth } => *depth,
                DisplayRow::SectionHeader { .. } => break,
            };
            if row_depth > base_depth {
                end = i;
            } else {
                break;
            }
        }
        end
    }

    /// Remove the InlineNew row at the given index and fix up selection.
    pub fn remove_inline_row(&mut self, row_index: usize) {
        if row_index < self.display_rows.len() {
            if let DisplayRow::InlineNew { depth } = self.display_rows[row_index] {
                let is_only_child_of_story = depth > 0
                    && row_index > 0
                    && matches!(
                        self.display_rows.get(row_index - 1),
                        Some(DisplayRow::Ticket { .. })
                    )
                    && !matches!(
                        self.display_rows.get(row_index + 1),
                        Some(
                            DisplayRow::Ticket { depth: d, .. }
                            | DisplayRow::Loading { depth: d }
                            | DisplayRow::Empty { depth: d }
                        ) if *d > 0
                    );
                if is_only_child_of_story {
                    self.display_rows[row_index] = DisplayRow::Empty { depth };
                } else {
                    self.display_rows.remove(row_index);
                }
            }
        }
        if !self.display_rows.is_empty() {
            self.selected_index = self.selected_index.min(self.display_rows.len() - 1);
        } else {
            self.selected_index = 0;
        }
    }

    pub fn start_search(&mut self) {
        // Focus is handled by caller
    }

    pub fn confirm_search(&mut self) {
        // Focus is handled by caller; filter persists to keep the list filtered.
    }

    pub fn cancel_search(
        &mut self,
        ticket_store: &TicketStore,
        issues: &[Issue],
        story_children: &HashMap<String, Vec<Issue>>,
    ) {
        let selected_key = self
            .selected_ticket(ticket_store)
            .map(|ticket| ticket.issue.key.clone());
        self.search_filter.clear();
        self.rebuild_display_rows(issues, story_children);
        if let Some(key) = selected_key {
            if let Some(position) = self.display_rows.iter().position(|row| match row {
                DisplayRow::Ticket { key: k, .. } => *k == key,
                _ => false,
            }) {
                self.selected_index = position;
            }
        }
        self.adjust_scroll_offset();
    }

    pub fn search_type_char(
        &mut self,
        ch: char,
        issues: &[Issue],
        story_children: &HashMap<String, Vec<Issue>>,
    ) {
        self.search_filter.push(ch);
        self.selected_index = 0;
        self.rebuild_display_rows(issues, story_children);
    }

    pub fn search_backspace(
        &mut self,
        issues: &[Issue],
        story_children: &HashMap<String, Vec<Issue>>,
    ) {
        self.search_filter.pop();
        self.selected_index = 0;
        self.rebuild_display_rows(issues, story_children);
    }

    /// Build the flattened display rows from the current issues list.
    pub fn rebuild_display_rows(
        &mut self,
        issues: &[Issue],
        story_children: &HashMap<String, Vec<Issue>>,
    ) {
        use std::collections::HashMap as StdMap;

        let is_searching = !self.search_filter.is_empty();

        let mut matcher = Matcher::new(Config::DEFAULT);
        let atoms: Vec<Atom> = self
            .search_filter
            .split_whitespace()
            .map(|word| {
                Atom::new(
                    word,
                    CaseMatching::Ignore,
                    Normalization::Smart,
                    AtomKind::Substring,
                    false,
                )
            })
            .collect();

        let issue_matches_search = |issue: &Issue, atoms: &[Atom], matcher: &mut Matcher| -> bool {
            let fields = [
                issue.key.as_str().to_owned(),
                issue.summary().unwrap_or_default().to_owned(),
                issue
                    .assignee()
                    .map(|u| u.display_name.clone())
                    .unwrap_or_default(),
                issue.status().map(|s| s.name.clone()).unwrap_or_default(),
            ];
            // Every word must match at least one field (but different words
            // can match different fields).
            atoms.iter().all(|atom| {
                fields.iter().any(|field| {
                    let mut buffer = Vec::new();
                    let haystack = Utf32Str::new(field, &mut buffer);
                    atom.score(haystack, matcher).is_some()
                })
            })
        };

        let matching_indices: Option<HashSet<usize>> = if !is_searching {
            None
        } else {
            // Also include story_children matches: when a lazily-loaded
            // child matches, surface its parent from the issues list.
            let matching_parent_keys: HashSet<String> = story_children
                .iter()
                .filter(|(_, children)| {
                    children
                        .iter()
                        .any(|c| issue_matches_search(c, &atoms, &mut matcher))
                })
                .map(|(key, _)| key.clone())
                .collect();

            Some(
                issues
                    .iter()
                    .enumerate()
                    .filter(|(_, issue)| {
                        issue_matches_search(issue, &atoms, &mut matcher)
                            || matching_parent_keys.contains(&issue.key)
                    })
                    .map(|(idx, _)| idx)
                    .collect(),
            )
        };

        let mut parent_groups: StdMap<String, (String, Vec<usize>)> = StdMap::new();
        let mut standalone_indices: Vec<usize> = Vec::new();
        let mut nested_issue_keys: HashSet<String> = HashSet::new();

        for (idx, issue) in issues.iter().enumerate() {
            if let Some(ref matching) = matching_indices {
                if !matching.contains(&idx) {
                    continue;
                }
            }
            if let Some(parent) = issue.parent() {
                nested_issue_keys.insert(issue.key.clone());
                let parent_key = parent.key.clone();
                let parent_summary = parent.summary().unwrap_or_default();
                let entry = parent_groups
                    .entry(parent_key)
                    .or_insert_with(|| (parent_summary, Vec::new()));
                entry.1.push(idx);
            } else {
                standalone_indices.push(idx);
            }
        }

        for (_, children) in parent_groups.values_mut() {
            children.sort_by(|a, b| {
                status_rank(&issues[*a])
                    .cmp(&status_rank(&issues[*b]))
                    .then_with(|| {
                        issue_created_str(&issues[*b]).cmp(&issue_created_str(&issues[*a]))
                    })
            });
        }
        let root_group_keys: HashSet<String> = parent_groups
            .keys()
            .filter(|key| !nested_issue_keys.contains(*key))
            .cloned()
            .collect();

        enum TopLevel {
            Standalone(usize),
            StoryGroup {
                key: String,
                parent_issue_idx: Option<usize>,
            },
        }

        let mut top_levels: Vec<TopLevel> = Vec::new();
        let mut emitted_parents: HashSet<String> = HashSet::new();

        for &idx in &standalone_indices {
            let issue_key = &issues[idx].key;
            if root_group_keys.contains(issue_key) {
                emitted_parents.insert(issue_key.clone());
                top_levels.push(TopLevel::StoryGroup {
                    key: issue_key.clone(),
                    parent_issue_idx: Some(idx),
                });
            } else {
                top_levels.push(TopLevel::Standalone(idx));
            }
        }

        for (parent_key, _) in &parent_groups {
            if !root_group_keys.contains(parent_key) || emitted_parents.contains(parent_key) {
                continue;
            }
            top_levels.push(TopLevel::StoryGroup {
                key: parent_key.clone(),
                parent_issue_idx: None,
            });
        }

        top_levels.sort_by(|a, b| {
            let rank_a = top_level_status_rank(a, issues, &parent_groups);
            let rank_b = top_level_status_rank(b, issues, &parent_groups);
            rank_a.cmp(&rank_b).then_with(|| {
                top_level_created(b, issues, &parent_groups).cmp(&top_level_created(
                    a,
                    issues,
                    &parent_groups,
                ))
            })
        });

        fn top_level_created(
            entry: &TopLevel,
            issues: &[Issue],
            parent_groups: &StdMap<String, (String, Vec<usize>)>,
        ) -> String {
            match entry {
                TopLevel::Standalone(idx) => issue_created_str(&issues[*idx]),
                TopLevel::StoryGroup {
                    key,
                    parent_issue_idx,
                    ..
                } => {
                    let parent_created =
                        parent_issue_idx.map(|idx| issue_created_str(&issues[idx]));
                    let child_max = parent_groups
                        .get(key)
                        .into_iter()
                        .flat_map(|(_, children)| children.iter())
                        .map(|idx| issue_created_str(&issues[*idx]))
                        .max();
                    parent_created
                        .into_iter()
                        .chain(child_max)
                        .max()
                        .unwrap_or_default()
                }
            }
        }

        fn top_level_status_rank(
            entry: &TopLevel,
            issues: &[Issue],
            parent_groups: &StdMap<String, (String, Vec<usize>)>,
        ) -> u8 {
            match entry {
                TopLevel::Standalone(idx) => status_rank(&issues[*idx]),
                TopLevel::StoryGroup {
                    key,
                    parent_issue_idx,
                    ..
                } => {
                    let child_min = parent_groups
                        .get(key)
                        .into_iter()
                        .flat_map(|(_, children)| children.iter())
                        .map(|idx| status_rank(&issues[*idx]))
                        .min()
                        .unwrap_or(u8::MAX);
                    let parent_rank = parent_issue_idx
                        .map(|idx| status_rank(&issues[idx]))
                        .unwrap_or(u8::MAX);
                    child_min.min(parent_rank)
                }
            }
        }

        let mut rows = Vec::new();
        let mut rendered_keys = HashSet::new();

        for section in [ListSection::Board, ListSection::Backlog] {
            let section_top_levels: Vec<&TopLevel> = top_levels
                .iter()
                .filter(|entry| match entry {
                    TopLevel::Standalone(idx) => {
                        let issue = &issues[*idx];
                        let backlog = is_backlog_status(issue);
                        let expandable = crate::issue::is_expandable(issue)
                            || story_children.contains_key(&issue.key)
                            || parent_groups.contains_key(&issue.key);
                        if expandable {
                            // When searching and the parent itself matched, use
                            // its own status for section placement instead of
                            // requiring matching children.
                            if is_searching
                                && matching_indices
                                    .as_ref()
                                    .map(|m| m.contains(idx))
                                    .unwrap_or(false)
                            {
                                return match section {
                                    ListSection::Board => !backlog,
                                    ListSection::Backlog => backlog,
                                };
                            }
                            return has_children_in_section(
                                &issue.key,
                                issues,
                                &parent_groups,
                                story_children,
                                section,
                            );
                        }
                        match section {
                            ListSection::Board => !backlog,
                            ListSection::Backlog => backlog,
                        }
                    }
                    TopLevel::StoryGroup { key, .. } => {
                        // When searching, a StoryGroup exists because a child
                        // matched — place it based on those children.  But also
                        // check if the parent key itself is in the issues list
                        // and matched the query; if so, fall back to its own
                        // status.
                        if has_children_in_section(
                            key,
                            issues,
                            &parent_groups,
                            story_children,
                            section,
                        ) {
                            return true;
                        }
                        if is_searching {
                            if let Some(parent_idx) = issues.iter().position(|i| i.key == *key) {
                                if matching_indices
                                    .as_ref()
                                    .map(|m| m.contains(&parent_idx))
                                    .unwrap_or(false)
                                {
                                    let backlog = is_backlog_status(&issues[parent_idx]);
                                    return match section {
                                        ListSection::Board => !backlog,
                                        ListSection::Backlog => backlog,
                                    };
                                }
                            }
                        }
                        false
                    }
                })
                .collect();

            if section_top_levels.is_empty() {
                continue;
            }

            let section_header_index = rows.len();
            rows.push(DisplayRow::SectionHeader { section, count: 0 });
            rendered_keys.clear();

            for entry in section_top_levels {
                match entry {
                    TopLevel::Standalone(idx) => {
                        let issue = &issues[*idx];
                        let issue_key = issue.key.clone();
                        if !rendered_keys.insert(issue_key.clone()) {
                            continue;
                        }
                        let backlog = is_backlog_status(issue);
                        let belongs = match section {
                            ListSection::Board => !backlog,
                            ListSection::Backlog => backlog,
                        };
                        let expandable = crate::issue::is_expandable(issue)
                            || story_children.contains_key(&issue_key)
                            || parent_groups.contains_key(&issue_key);
                        if expandable {
                            if section != ListSection::Board
                                && !story_children.contains_key(&issue_key)
                                && !self.loading_children.contains(&issue_key)
                                && !self.has_ticket_row(&issue_key)
                            {
                                self.collapsed_stories.insert((issue_key.clone(), section));
                            }
                            rows.push(DisplayRow::Ticket {
                                key: issue_key.clone(),
                                depth: 0,
                            });
                            if !self
                                .collapsed_stories
                                .contains(&(issue_key.clone(), section))
                            {
                                self.append_nested_children(
                                    &issue_key,
                                    1,
                                    &mut rows,
                                    issues,
                                    &parent_groups,
                                    story_children,
                                    &mut rendered_keys,
                                    section,
                                );
                            }
                        } else if belongs {
                            rows.push(DisplayRow::Ticket {
                                key: issue.key.clone(),
                                depth: 0,
                            });
                        }
                    }
                    TopLevel::StoryGroup {
                        key,
                        parent_issue_idx,
                    } => {
                        if !rendered_keys.insert(key.clone()) {
                            continue;
                        }
                        if section != ListSection::Board
                            && !has_children_in_section(
                                key,
                                issues,
                                &parent_groups,
                                story_children,
                                section,
                            )
                            && !self.loading_children.contains(key.as_str())
                            && !self.has_ticket_row(key)
                        {
                            self.collapsed_stories.insert((key.clone(), section));
                        }
                        rows.push(DisplayRow::Ticket {
                            key: key.clone(),
                            depth: 0,
                        });
                        if !self.collapsed_stories.contains(&(key.clone(), section)) {
                            let _ = parent_issue_idx;
                            self.append_nested_children(
                                &key,
                                1,
                                &mut rows,
                                issues,
                                &parent_groups,
                                story_children,
                                &mut rendered_keys,
                                section,
                            );
                        }
                    }
                }
            }

            // Count leaf ticket rows in this section (exclude group headers).
            let section_rows = &rows[section_header_index + 1..];
            let section_issue_count = section_rows
                .iter()
                .enumerate()
                .filter(|(i, row)| {
                    let DisplayRow::Ticket { depth, .. } = row else {
                        return false;
                    };
                    // A ticket is a leaf if the next row is NOT deeper.
                    let next_deeper = section_rows.get(i + 1).map_or(false, |next| match next {
                        DisplayRow::Ticket { depth: d, .. }
                        | DisplayRow::Loading { depth: d }
                        | DisplayRow::Empty { depth: d } => *d > *depth,
                        _ => false,
                    });
                    !next_deeper
                })
                .count();
            if let DisplayRow::SectionHeader { count, .. } = &mut rows[section_header_index] {
                *count = section_issue_count;
            }
        }

        self.display_rows = rows;
        if !self.display_rows.is_empty() && self.selected_index >= self.display_rows.len() {
            self.selected_index = self.display_rows.len() - 1;
        }
        self.skip_section_headers(1);
    }

    /// Append children for a nested story header.
    fn append_nested_children(
        &mut self,
        parent_key: &str,
        depth: u8,
        rows: &mut Vec<DisplayRow>,
        issues: &[Issue],
        parent_groups: &HashMap<String, (String, Vec<usize>)>,
        story_children: &HashMap<String, Vec<Issue>>,
        rendered_keys: &mut HashSet<String>,
        section: ListSection,
    ) {
        let mut rendered_child = false;

        if let Some((_, grouped_children)) = parent_groups.get(parent_key) {
            for &idx in grouped_children {
                let child = &issues[idx];
                let child_key = child.key.clone();
                let expandable = crate::issue::is_expandable(child)
                    || story_children.contains_key(&child_key)
                    || parent_groups.contains_key(&child_key);
                if expandable {
                    // Expandable children: section placement inferred from their own children.
                    if !has_children_in_section(
                        &child_key,
                        issues,
                        parent_groups,
                        story_children,
                        section,
                    ) {
                        rendered_keys.remove(&child_key);
                        continue;
                    }
                } else {
                    let wrong_section = match section {
                        ListSection::Board => is_backlog_status(child),
                        ListSection::Backlog => !is_backlog_status(child),
                    };
                    if wrong_section {
                        rendered_keys.remove(&child.key);
                        continue;
                    }
                }
                if !rendered_keys.insert(child_key.clone()) {
                    continue;
                }
                rendered_child = true;
                if expandable {
                    if section != ListSection::Board
                        && !has_children_in_section(
                            &child_key,
                            issues,
                            parent_groups,
                            story_children,
                            section,
                        )
                        && !self.loading_children.contains(&child_key)
                        && !self.has_ticket_row(&child_key)
                    {
                        self.collapsed_stories.insert((child_key.clone(), section));
                    }
                    rows.push(DisplayRow::Ticket {
                        key: child_key.clone(),
                        depth,
                    });
                    if !self
                        .collapsed_stories
                        .contains(&(child_key.clone(), section))
                    {
                        self.append_nested_children(
                            &child_key,
                            depth + 1,
                            rows,
                            issues,
                            parent_groups,
                            story_children,
                            rendered_keys,
                            section,
                        );
                    }
                } else {
                    rows.push(DisplayRow::Ticket {
                        key: child.key.clone(),
                        depth,
                    });
                }
            }
        }

        if let Some(children) = story_children.get(parent_key) {
            for child in children.iter() {
                let child_key = child.key.clone();
                let expandable = crate::issue::is_expandable(child)
                    || story_children.contains_key(&child_key)
                    || parent_groups.contains_key(&child_key);
                if expandable {
                    if !has_children_in_section(
                        &child_key,
                        issues,
                        parent_groups,
                        story_children,
                        section,
                    ) {
                        rendered_keys.remove(&child_key);
                        continue;
                    }
                } else {
                    let wrong_section = match section {
                        ListSection::Board => is_backlog_status(child),
                        ListSection::Backlog => !is_backlog_status(child),
                    };
                    if wrong_section {
                        rendered_keys.remove(&child.key);
                        continue;
                    }
                }
                if !rendered_keys.insert(child_key.clone()) {
                    continue;
                }
                rendered_child = true;
                if expandable {
                    if section != ListSection::Board
                        && !has_children_in_section(
                            &child_key,
                            issues,
                            parent_groups,
                            story_children,
                            section,
                        )
                        && !self.loading_children.contains(&child_key)
                        && !self.has_ticket_row(&child_key)
                    {
                        self.collapsed_stories.insert((child_key.clone(), section));
                    }
                    rows.push(DisplayRow::Ticket {
                        key: child_key.clone(),
                        depth,
                    });
                    if !self
                        .collapsed_stories
                        .contains(&(child_key.clone(), section))
                    {
                        self.append_nested_children(
                            &child_key,
                            depth + 1,
                            rows,
                            issues,
                            parent_groups,
                            story_children,
                            rendered_keys,
                            section,
                        );
                    }
                } else {
                    rows.push(DisplayRow::Ticket {
                        key: child.key.clone(),
                        depth,
                    });
                }
            }
        }

        if rendered_child {
            return;
        }

        // When filtering by section, children may exist in the other section.
        // Only suppress placeholders when children data has actually been loaded.
        let children_data_exists =
            parent_groups.contains_key(parent_key) || story_children.contains_key(parent_key);
        if matches!(section, ListSection::Backlog) && children_data_exists {
            return;
        }

        // Only show the loading spinner when no children data exists yet.
        // When a refetch is in-flight but we still have stale data, keep
        // showing the old children to avoid a visible flicker.
        let has_no_data =
            !parent_groups.contains_key(parent_key) && !story_children.contains_key(parent_key);
        if has_no_data {
            rows.push(DisplayRow::Loading { depth });
            return;
        }

        if story_children
            .get(parent_key)
            .map(|children| children.is_empty())
            .unwrap_or(false)
        {
            rows.push(DisplayRow::Empty { depth });
        }
    }

    fn is_section_header(&self, index: usize) -> bool {
        matches!(
            self.display_rows.get(index),
            Some(DisplayRow::SectionHeader { .. })
        )
    }

    fn skip_section_headers(&mut self, direction: isize) {
        let len = self.display_rows.len();
        while self.is_section_header(self.selected_index) {
            let next = self.selected_index as isize + direction;
            if next < 0 || next >= len as isize {
                let other = self.selected_index as isize - direction;
                if other >= 0 && (other as usize) < len {
                    self.selected_index = other as usize;
                }
                break;
            }
            self.selected_index = next as usize;
        }
    }

    pub fn move_selection_down(&mut self) {
        if self.display_rows.is_empty() {
            self.selected_index = 0;
            self.skip_section_headers(1);
            return;
        }
        move_selected_index(&mut self.selected_index, self.display_rows.len(), 1);
        self.skip_section_headers(1);
        self.adjust_scroll_offset();
    }

    pub fn move_selection_up(&mut self) {
        move_selected_index(&mut self.selected_index, self.display_rows.len(), -1);
        self.skip_section_headers(-1);
        self.adjust_scroll_offset();
    }

    pub fn move_selection_to_end(&mut self) {
        if self.display_rows.is_empty() {
            return;
        }
        self.selected_index = self.display_rows.len() - 1;
        self.skip_section_headers(-1);
        self.adjust_scroll_offset();
    }

    pub fn move_selection_by(&mut self, delta: isize) {
        if self.display_rows.is_empty() {
            return;
        }
        move_selected_index(&mut self.selected_index, self.display_rows.len(), delta);
        self.skip_section_headers(delta.signum());
        self.adjust_scroll_offset();
    }

    pub fn scroll_viewport(&mut self, delta: isize) {
        if self.display_rows.is_empty() {
            return;
        }
        let height = self.area_height as usize;
        let max_offset = self.display_rows.len().saturating_sub(height);
        let new_offset =
            (self.scroll_offset as isize + delta).clamp(0, max_offset as isize) as usize;
        self.scroll_offset = new_offset;

        let last = self.display_rows.len() - 1;
        if self.selected_index < new_offset {
            self.selected_index = new_offset;
        } else if self.selected_index >= new_offset + height {
            self.selected_index = (new_offset + height - 1).min(last);
        }
        self.skip_section_headers(delta.signum());
    }

    pub fn adjust_scroll_offset(&mut self) {
        adjust_scroll_offset(
            &mut self.scroll_offset,
            self.selected_index,
            self.area_height,
            self.display_rows.len(),
        );
    }

    pub fn render(
        &mut self,
        frame: &mut Frame,
        area: ratatui::layout::Rect,
        ctx: &ListRenderContext,
    ) {
        self.area_height = area.height.saturating_sub(1);

        let mut current_section: Option<ListSection> = None;
        let mut row_data: Vec<(CellMap, Style)> = Vec::with_capacity(self.display_rows.len());
        for (row_idx, display_row) in self.display_rows.iter().enumerate() {
            let cells = match display_row {
                DisplayRow::SectionHeader { section, count } => {
                    current_section = Some(*section);
                    row::section_header_row(section, *count)
                }
                DisplayRow::Ticket { key, depth } => {
                    let ticket = ctx.ticket_store.get(key);
                    let is_group_header = ticket
                        .map(|t| crate::issue::is_expandable(&t.issue))
                        .unwrap_or(false)
                        || self
                            .display_rows
                            .get(row_idx + 1)
                            .map(|next| match next {
                                DisplayRow::Ticket { depth: d, .. }
                                | DisplayRow::Loading { depth: d }
                                | DisplayRow::Empty { depth: d } => *d > *depth,
                                _ => false,
                            })
                            .unwrap_or(false);
                    if is_group_header {
                        let collapsed = current_section
                            .map(|s| self.collapsed_stories.contains(&(key.clone(), s)))
                            .unwrap_or(false);
                        let has_pending_import = self.pending_import_keys.contains(key);
                        let ticket =
                            ticket.unwrap_or_else(|| panic!("missing ticket for key {key}"));
                        row::story_header_row(ticket, collapsed, *depth, has_pending_import)
                    } else {
                        let ticket =
                            ticket.unwrap_or_else(|| panic!("missing ticket for key {key}"));
                        row::issue_row(ctx, &self.pending_import_keys, ticket, *depth)
                    }
                }
                DisplayRow::InlineNew { depth } => row::inline_new_row(ctx.inline_new, *depth),
                DisplayRow::Loading { depth } => {
                    row::loading_row(ctx.animation.spinner_tick, *depth)
                }
                DisplayRow::Empty { depth } => row::empty_row(*depth),
            };
            row_data.push(cells);
        }

        let constraints = [
            Constraint::Min(10),
            Constraint::Length(max_col_width(&row_data, "Status").min(14)),
            Constraint::Length(max_col_width(&row_data, "Dev").min(12)),
            Constraint::Length(max_col_width(&row_data, "◷").min(4)),
            Constraint::Length(max_col_width(&row_data, "PR").min(8)),
            Constraint::Length(max_col_width(&row_data, "CI").min(14)),
            Constraint::Length(max_col_width(&row_data, "Repo").min(24)),
        ];

        let mut state = TableState::default()
            .with_offset(self.scroll_offset)
            .with_selected(Some(self.selected_index));

        let rows: Vec<Row> = row_data
            .into_iter()
            .map(|(mut cells, style)| {
                let ordered: Vec<Cell> = COLUMNS
                    .iter()
                    .map(|col| Cell::from(cells.remove(col).unwrap_or_default()))
                    .collect();
                Row::new(ordered).style(style)
            })
            .collect();

        let table = Table::new(rows, constraints)
            .header(
                Row::new(COLUMNS.iter().copied())
                    .style(
                        Style::default()
                            .fg(Theme::Muted)
                            .add_modifier(Modifier::BOLD),
                    )
                    .bottom_margin(0),
            )
            .column_spacing(2)
            .row_highlight_style(
                Style::default()
                    .bg(Theme::Selection)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("▌")
            .highlight_spacing(HighlightSpacing::Always)
            .block(Block::default().style(Style::default().bg(Theme::Panel)));
        frame.render_stateful_widget(table, area, &mut state);
        self.scroll_offset = state.offset();
    }
}

pub async fn update(app: &mut crate::app::AppView, key_event: KeyEvent) {
    match app.input_focus {
        crate::app::InputFocus::List => {
            let previous_was_g = app.previous_key == Some(KeyCode::Char('g'));

            if key_event.modifiers.contains(KeyModifiers::CONTROL) {
                match key_event.code {
                    KeyCode::Char('d') | KeyCode::Char('D') => {
                        app.list
                            .move_selection_by(app.list.area_height as isize / 2);
                        app.schedule_prefetch();
                    }
                    KeyCode::Char('u') | KeyCode::Char('U') => {
                        app.list
                            .move_selection_by(-(app.list.area_height as isize / 2));
                        app.schedule_prefetch();
                    }
                    _ => {}
                }
                return;
            }

            match key_event.code {
                KeyCode::Char(c) => {
                    if previous_was_g && c == 'g' {
                        app.list.selected_index = 0;
                        app.list.skip_section_headers(1);
                        app.list.adjust_scroll_offset();
                        app.schedule_prefetch();
                        return;
                    }

                    match c {
                        'b' => show_branch_diff_dialog(app),
                        'j' => {
                            app.list.move_selection_down();
                            app.schedule_prefetch();
                        }
                        'k' => {
                            app.list.move_selection_up();
                            app.schedule_prefetch();
                        }
                        'G' => {
                            app.list.move_selection_to_end();
                            app.schedule_prefetch();
                        }
                        'p' => show_pick_up_dialog(app),
                        'o' => match open_selected_pr_in_browser(app).await {
                            Ok(_) => {}
                            Err(err) => app.status_bar.set_error(format!("{err}")),
                        },
                        't' => match open_selected_issue_in_browser(app).await {
                            Ok(_) => {}
                            Err(err) => app
                                .status_bar
                                .set_error(format!("Failed to open issue: {err}")),
                        },
                        'n' => {
                            let project_key = derive_project_key(app);
                            app.list.start_inline_new(project_key);
                            app.input_focus = crate::app::InputFocus::InlineNew;
                        }
                        'a' => open_label_picker(app),
                        'F' => open_jira_filter_picker(app),
                        'r' => {
                            app.loading = true;
                            app.spawn_refresh();
                        }
                        'S' => spawn_toggle_story_type(app),
                        'f' => show_finish_dialog(app),
                        '/' => {
                            app.list.start_search();
                            app.input_focus = crate::app::InputFocus::Search;
                        }
                        'V' => spawn_approve_merge(app),
                        'c' => open_ci_log_popup(app),
                        'e' => spawn_openspec_propose(app),
                        'i' => open_import_tasks_popup(app),
                        '?' => {
                            app.help_overlay = Some(crate::ui::HelpOverlayView::default());
                            app.input_focus = crate::app::InputFocus::HelpOverlay;
                        }
                        'h' => {
                            app.list.collapse_story(
                                &app.ticket_store,
                                &app.issues,
                                &app.story_children,
                            );
                            app.save_cache();
                        }
                        'l' => {
                            if let Some(key) = app.list.expand_story(
                                &app.ticket_store,
                                &app.issues,
                                &app.story_children,
                            ) {
                                app.spawn_fetch_children(&key);
                            }
                            app.save_cache();
                        }
                        ' ' => {
                            if let Some(key) = app.list.toggle_story_collapse(
                                &app.ticket_store,
                                &app.issues,
                                &app.story_children,
                            ) {
                                app.spawn_fetch_children(&key);
                            }
                            app.save_cache();
                        }
                        _ => {}
                    }
                }
                KeyCode::Esc => {
                    if !app.list.search_filter.is_empty() {
                        app.list
                            .cancel_search(&app.ticket_store, &app.issues, &app.story_children);
                        app.input_focus = crate::app::InputFocus::List;
                    }
                }
                KeyCode::Down => {
                    app.list.move_selection_down();
                    app.schedule_prefetch();
                }
                KeyCode::Up => {
                    app.list.move_selection_up();
                    app.schedule_prefetch();
                }
                _ => {}
            }
        }
        crate::app::InputFocus::Search => match key_event.code {
            KeyCode::Esc => {
                app.list
                    .cancel_search(&app.ticket_store, &app.issues, &app.story_children);
                app.input_focus = crate::app::InputFocus::List;
            }
            KeyCode::Enter => {
                app.list.confirm_search();
                app.input_focus = crate::app::InputFocus::List;
            }
            KeyCode::Backspace => {
                app.list.search_backspace(&app.issues, &app.story_children);
            }
            KeyCode::Char(c) => {
                if !key_event
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                {
                    app.list
                        .search_type_char(c, &app.issues, &app.story_children);
                }
            }
            _ => {}
        },
        crate::app::InputFocus::InlineNew => {
            if key_event.modifiers.contains(KeyModifiers::CONTROL) {
                match key_event.code {
                    KeyCode::Char('s') | KeyCode::Char('S') => {
                        spawn_submit_inline_new(app);
                        return;
                    }
                    _ => {}
                }
            }

            match key_event.code {
                KeyCode::Esc => {
                    app.list.cancel_inline_new();
                    app.input_focus = crate::app::InputFocus::List;
                }
                KeyCode::Enter => {
                    spawn_submit_inline_new(app);
                }
                KeyCode::Backspace => {
                    if let Some(state) = app.list.inline_new.as_mut() {
                        state.summary.pop();
                    }
                }
                KeyCode::Char(c) => {
                    if !key_event
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                    {
                        if let Some(state) = app.list.inline_new.as_mut() {
                            state.summary.push(c);
                        }
                    }
                }
                _ => {}
            }
        }
        _ => {}
    }
}

/// Show confirmation dialog before picking up an issue.
fn show_pick_up_dialog(app: &mut AppView) {
    let Some(ticket) = app.list.selected_ticket(&app.ticket_store) else {
        return;
    };
    let issue_key = ticket.issue.key.clone();
    let issue_summary = ticket.issue.summary().unwrap_or_default();
    let issue_description = crate::issue::description(&ticket.issue).unwrap_or_default();
    let repo_path = ticket.repos.first().map(|entry| entry.path.clone());
    let ancestors =
        crate::issue::ancestors_from_sources(&ticket.issue, &app.issues, &app.story_children);

    app.confirm_dialog = Some(ConfirmDialogView {
        action: ConfirmAction::PickUp {
            issue_key,
            issue_summary,
            issue_description,
            repo_path,
            my_account_id: app.my_account_id.clone(),
            ancestors,
        },
    });
    app.input_focus = InputFocus::ConfirmDialog;
}

/// Show confirmation dialog before opening branch diff.
fn show_branch_diff_dialog(app: &mut AppView) {
    let Some(ticket) = app.list.selected_ticket(&app.ticket_store) else {
        return;
    };
    let issue_key = ticket.issue.key.clone();
    let repo_path = match ticket.repos.first() {
        Some(entry) => entry.path.clone(),
        None => {
            app.status_bar
                .set_warning(format!("Cannot open diff for {issue_key}: no tagged repo"));
            return;
        }
    };

    app.confirm_dialog = Some(ConfirmDialogView {
        action: ConfirmAction::BranchDiff {
            issue_key,
            repo_path,
        },
    });
    app.input_focus = InputFocus::ConfirmDialog;
}

/// Spawn approve + auto-merge for the selected issue's PR.
fn spawn_approve_merge(app: &mut AppView) {
    let Some(ticket) = app.list.selected_ticket(&app.ticket_store) else {
        return;
    };
    let issue_key = ticket.issue.key.clone();
    let Some(pr) = ticket.pr.as_ref() else {
        app.status_bar
            .set_warning(format!("No PR found for {issue_key}"));
        return;
    };

    actions::approve_merge::spawn(app.message_tx.clone(), pr.repo_slug.clone(), pr.number);
}

/// Show confirmation dialog before finishing an issue.
fn show_finish_dialog(app: &mut AppView) {
    let Some(ticket) = app.list.selected_ticket(&app.ticket_store) else {
        return;
    };
    let issue_key = ticket.issue.key.clone();
    let issue_summary = ticket.issue.summary().unwrap_or_default();
    let repo_path = match ticket.repos.first() {
        Some(entry) => entry.path.clone(),
        None => {
            app.status_bar
                .set_warning(format!("Cannot finish {issue_key}: no tagged repo"));
            return;
        }
    };

    app.confirm_dialog = Some(ConfirmDialogView {
        action: ConfirmAction::Finish {
            issue_key,
            issue_summary,
            repo_path,
        },
    });
    app.input_focus = InputFocus::ConfirmDialog;
}

/// Spawn issue type toggle: Task → Story, or Story → Task if it has no children.
fn spawn_toggle_story_type(app: &mut AppView) {
    let Some(ticket) = app.list.selected_ticket(&app.ticket_store) else {
        return;
    };
    let issue_type_name = ticket
        .issue
        .issue_type()
        .map(|t| t.name.to_lowercase())
        .unwrap_or_default();
    let issue_key = ticket.issue.key.clone();

    if issue_type_name.contains("story") || issue_type_name.contains("epic") {
        let has_children = app
            .story_children
            .get(&issue_key)
            .map(|children| !children.is_empty())
            .unwrap_or(false);
        if has_children {
            app.status_bar
                .set_warning(format!("{issue_key} has children — remove them first"));
            return;
        }
        actions::convert_to_story::spawn(
            app.message_tx.clone(),
            app.client.clone(),
            issue_key,
            "Task",
        );
        return;
    }

    actions::convert_to_story::spawn(
        app.message_tx.clone(),
        app.client.clone(),
        issue_key,
        "Story",
    );
}

/// Spawn inline new issue creation in background.
fn spawn_submit_inline_new(app: &mut AppView) {
    let Some(state) = app.list.inline_new.take() else {
        return;
    };
    let summary = state.summary.trim().to_string();
    if summary.is_empty() {
        app.list.remove_inline_row(state.row_index);
        app.input_focus = InputFocus::List;
        app.status_bar.set_warning("Summary cannot be empty");
        return;
    }

    app.input_focus = InputFocus::List;
    let Some(jql) = app.current_issue_jql() else {
        app.status_bar.set_warning("Select a Jira project first");
        return;
    };
    actions::create_inline_issue::spawn(
        app.message_tx.clone(),
        app.client.clone(),
        jql,
        state.project_key,
        summary,
        state.parent_key,
    );
}

fn derive_project_key(app: &AppView) -> String {
    if let Some(project_key) = app.current_project_key() {
        return project_key.to_string();
    }

    if let Some(project_key) = app
        .list
        .selected_ticket(&app.ticket_store)
        .and_then(|ticket| ticket.issue.project())
        .map(|project| project.key)
    {
        return project_key;
    }

    app.issues
        .first()
        .and_then(|issue| issue.project())
        .map(|project| project.key)
        .unwrap_or_else(|| "WORK".to_string())
}

fn open_label_picker(app: &mut AppView) {
    if app.repo_entries.is_empty() {
        app.status_bar
            .set_warning("No repositories found in REPOS_DIR");
        return;
    }
    app.label_picker = Some(LabelPickerView::default());
    app.input_focus = InputFocus::LabelPicker;
}

fn open_jira_filter_picker(app: &mut AppView) {
    crate::ui::filter_picker::open(app);
}

fn open_ci_log_popup(app: &mut AppView) {
    let Some(ticket) = app.list.selected_ticket(&app.ticket_store) else {
        app.status_bar.set_warning("No issue selected");
        return;
    };
    let issue_key = ticket.issue.key.clone();
    let Some(pr) = ticket.pr.as_ref() else {
        app.status_bar
            .set_warning(format!("No linked PR for {issue_key}"));
        return;
    };
    if pr.check_runs.is_empty() {
        app.status_bar
            .set_warning(format!("No CI checks for {issue_key}"));
        return;
    }
    app.ci_log_popup.open();
    app.input_focus = InputFocus::CiLogPopup;
    spawn_ci_log_fetch(app, &issue_key);
}

/// Spawn CI log fetch if logs aren't already cached or in-flight.
fn spawn_ci_log_fetch(app: &mut AppView, issue_key: &str) {
    let Some(pr) = app.github_prs.get(issue_key) else {
        return;
    };
    if !app.ci_log_popup.start_loading(issue_key) {
        return;
    }
    actions::fetch_ci_logs::spawn(
        app.message_tx.clone(),
        issue_key.to_string(),
        pr.repo_slug.clone(),
        pr.check_runs.clone(),
    );
}

/// Scan openspec changes for pending import tasks.
fn open_import_tasks_popup(app: &mut AppView) {
    let Some(ticket) = app.list.selected_ticket(&app.ticket_store) else {
        return;
    };
    let issue_key = ticket.issue.key.clone();
    let issue_type_name = ticket
        .issue
        .issue_type()
        .map(|t| t.name.clone())
        .unwrap_or_default();
    let project_key = derive_project_key(app);

    let tasks_path = match actions::import_tasks::find_tasks_json(&app.config.repos_dir, &issue_key)
    {
        Ok(path) => path,
        Err(err) => {
            app.status_bar.set_error(format!("{err}"));
            return;
        }
    };

    let tasks = match actions::import_tasks::load_tasks(&tasks_path) {
        Ok(tasks) => tasks,
        Err(err) => {
            app.status_bar.set_error(format!("{err}"));
            return;
        }
    };

    let pending_count = tasks.iter().filter(|t| t.key.is_none()).count();
    if pending_count == 0 {
        app.status_bar.set_warning("All tasks already imported");
        return;
    }

    app.import_tasks_popup = Some(ImportTasksView {
        tasks,
        tasks_path,
        issue_key,
        issue_type_name,
        project_key,
        scroll: 0,
    });
    app.input_focus = InputFocus::ImportTasksPopup;
}

/// Open an opencode session to propose an openspec change for the selected issue.
fn spawn_openspec_propose(app: &mut AppView) {
    let Some(ticket) = app.list.selected_ticket(&app.ticket_store) else {
        return;
    };
    let issue_key = ticket.issue.key.clone();
    let issue_summary = ticket.issue.summary().unwrap_or_default();
    let issue_description = crate::issue::description(&ticket.issue).unwrap_or_default();

    let repo_slugs: Vec<String> = ticket
        .repos
        .iter()
        .filter_map(|entry| entry.github_slug.clone())
        .collect();

    let ancestors =
        crate::issue::ancestors_from_sources(&ticket.issue, &app.issues, &app.story_children);

    actions::openspec_propose::spawn(
        app.message_tx.clone(),
        app.config.repos_dir.clone(),
        issue_key,
        issue_summary,
        issue_description,
        ancestors,
        repo_slugs,
    );
}

async fn open_selected_issue_in_browser(app: &mut AppView) -> Result<()> {
    let issue_key = match app.list.selected_ticket(&app.ticket_store) {
        Some(ticket) => ticket.issue.key.clone(),
        None => return Err(eyre!("No issue selected")),
    };

    let url = format!("{}/browse/{}", app.config.jira.jira_url, issue_key);
    open_url_in_browser(&url).await?;
    Ok(())
}

async fn open_selected_pr_in_browser(app: &mut AppView) -> Result<()> {
    let Some(ticket) = app.list.selected_ticket(&app.ticket_store) else {
        return Err(eyre!("No issue selected"));
    };
    let issue_key = ticket.issue.key.clone();
    let pr = ticket
        .pr
        .as_ref()
        .ok_or_else(|| eyre!("No PR found for {issue_key}"))?;

    let url = pr.url.clone();
    open_url_in_browser(&url).await?;
    Ok(())
}

async fn open_url_in_browser(url: &str) -> Result<()> {
    let mut command = if cfg!(target_os = "macos") {
        let mut command = Command::new("open");
        command.arg(url);
        command
    } else if cfg!(target_os = "windows") {
        let mut command = Command::new("cmd");
        command.arg("/C").arg("start").arg("").arg(url);
        command
    } else {
        let mut command = Command::new("xdg-open");
        command.arg(url);
        command
    };

    let output = command.output().await?;
    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if stderr.is_empty() {
        return Err(eyre!("Failed to open browser"));
    }

    Err(eyre!(stderr))
}

/// Numeric rank for sorting issues by status.
fn status_rank(issue: &Issue) -> u8 {
    const ORDER: &[&str] = &["review", "progress", "rejected", "plan", "proposed"];
    let name = issue
        .status()
        .map(|s| s.name)
        .unwrap_or_default()
        .to_lowercase();
    ORDER
        .iter()
        .position(|&keyword| name.contains(keyword))
        .map(|i| i as u8)
        .unwrap_or(ORDER.len() as u8)
}

fn issue_created_str(issue: &Issue) -> String {
    issue
        .field::<String>("created")
        .and_then(|r| r.ok())
        .unwrap_or_default()
}
