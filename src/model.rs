use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Person {
    pub account_id: String,
    pub name: String,
}

#[derive(Clone, Debug, Default)]
pub struct Parent {
    pub key: String,
    pub summary: String,
}

#[derive(Clone, Debug, Default)]
pub struct Issue {
    pub key: String,
    pub summary: String,
    pub status: String,
    pub status_id: String,
    pub is_subtask: bool,
    pub is_epic: bool,
    /// Whether the issue is a row of the list. Epics are context for their tasks, and an issue
    /// with subtasks is represented by those subtasks; both are loaded but not listed.
    pub listed: bool,
    /// What the issue is filed under: its parent, or for a subtask its parent's parent.
    pub epic: Option<Parent>,
    /// The board column the issue is in, or the backlog.
    pub section: String,
    pub assignee: Option<Person>,
    pub labels: Vec<String>,
    pub parent: Option<Parent>,
    pub updated: Option<DateTime<Utc>>,
    pub description: String,
}

impl Issue {
    pub fn in_progress(&self) -> bool {
        self.status.to_lowercase().contains("progress")
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CheckState {
    Passed,
    Failed,
    Running,
    Skipped,
}

#[derive(Clone, Debug)]
pub struct Check {
    pub name: String,
    pub state: CheckState,
    /// GitHub Actions run the check belongs to; absent for checks from other providers.
    pub run_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Review {
    Approved,
    ChangesRequested,
    /// Nobody has approved or requested changes yet.
    #[default]
    Awaited,
}

#[derive(Clone, Debug, Default)]
pub struct Pr {
    /// GitHub's node id, for fetching this pull request again without searching.
    pub id: String,
    pub number: u64,
    pub url: String,
    pub repo_slug: String,
    pub branch: String,
    pub author: String,
    pub is_draft: bool,
    pub review: Review,
    pub has_conflicts: bool,
    pub is_mine: bool,
    /// I have not reviewed it yet, or a new review was requested from me since.
    pub awaits_my_review: bool,
    pub auto_merge: bool,
    pub additions: u64,
    pub deletions: u64,
    pub unresolved: usize,
    pub checks: Vec<Check>,
}

impl Pr {
    pub fn repo(&self) -> &str {
        self.repo_slug.rsplit('/').next().unwrap_or(&self.repo_slug)
    }

    /// Whether there is something I can do to move this pull request forward: on my own,
    /// resolve conflicts, fix CI or answer review feedback; on someone else's, review it.
    pub fn is_actionable(&self) -> bool {
        if self.is_mine {
            return self.has_conflicts
                || self.ci() == Some(CheckState::Failed)
                || self.review == Review::ChangesRequested
                || self.unresolved > 0;
        }
        !self.is_draft && self.awaits_my_review
    }

    /// The overall CI state: failed wins over running, running over passed.
    pub fn ci(&self) -> Option<CheckState> {
        [CheckState::Failed, CheckState::Running, CheckState::Passed]
            .into_iter()
            .find(|state| self.checks.iter().any(|check| check.state == *state))
    }
}

#[derive(Default)]
pub struct Data {
    pub issues: Vec<Issue>,
    /// The board columns up to and including Review, then the backlog.
    pub sections: Vec<String>,
    /// Open pull requests by issue key.
    pub prs: HashMap<String, Pr>,
    /// Workspace URLs by issue key.
    pub workspaces: HashMap<String, String>,
    pub me: Person,
    pub github_login: String,
}

/// The issue key a branch name starts with: `INI-12-fix-it` gives `INI-12`.
pub fn issue_key_of(branch: &str) -> Option<String> {
    let (project, rest) = branch.split_once('-')?;
    let number: String = rest.chars().take_while(char::is_ascii_digit).collect();
    let is_key = !project.is_empty()
        && project.chars().all(|c| c.is_ascii_alphabetic())
        && !number.is_empty();
    is_key.then(|| format!("{}-{number}", project.to_uppercase()))
}

/// Lowercased with every run of other characters collapsed into a dash, so `adminPanel_Backend`
/// and `adminpanel-backend` compare equal as far as labels go.
pub fn normalize(name: &str) -> String {
    let mut normalized = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            normalized.push(c.to_ascii_lowercase());
        } else if !normalized.ends_with('-') {
            normalized.push('-');
        }
    }
    normalized.trim_matches('-').to_owned()
}

/// Whether every whitespace-separated term of `query` occurs in `text`, ignoring case.
pub fn matches(text: &str, query: &str) -> bool {
    let text = text.to_lowercase();
    query
        .to_lowercase()
        .split_whitespace()
        .all(|term| text.contains(term))
}

pub const BACKLOG: &str = "Backlog";

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    /// A board column or the backlog.
    Section,
    /// The shared parent of the backlog issues below it.
    Parent,
    Issue,
}

/// One line of the list.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub kind: Kind,
    /// The issue key, or the name of the section.
    pub key: String,
    pub title: String,
    /// Index into the issue list; absent for sections and for parents that are not loaded.
    pub issue: Option<usize>,
    /// Number of issues in a section or under a parent.
    pub count: usize,
    /// Whether the issue sits under a parent row.
    pub indented: bool,
    pub section: String,
}

/// The collapsed-set entry of a section.
pub fn section_fold(section: &str) -> String {
    format!("§{section}")
}

/// The listed issues per section, in their given order. The board columns are flat; the backlog is
/// grouped by epic, since that is where work is still being organised. A search also matches
/// on the name of an issue's parent, and ignores what is collapsed.
pub fn rows(
    issues: &[Issue],
    sections: &[String],
    collapsed: &HashSet<String>,
    query: &str,
) -> Vec<Row> {
    let searchable = |issue: &Issue| {
        let assignee = issue
            .assignee
            .as_ref()
            .map_or("", |person| person.name.as_str());
        let parent = issue.epic.as_ref().map_or("", |epic| epic.summary.as_str());
        format!(
            "{} {} {} {assignee} {parent}",
            issue.key, issue.summary, issue.status
        )
    };
    let mut rows = Vec::new();
    for section in sections {
        let members: Vec<usize> = issues
            .iter()
            .enumerate()
            .filter(|(_, issue)| issue.section == *section && issue.listed)
            .filter(|(_, issue)| matches(&searchable(issue), query))
            .map(|(index, _)| index)
            .collect();
        if members.is_empty() {
            continue;
        }
        rows.push(Row {
            kind: Kind::Section,
            key: section.clone(),
            title: section.clone(),
            issue: None,
            count: members.len(),
            indented: false,
            section: section.clone(),
        });
        if query.is_empty() && collapsed.contains(&section_fold(section)) {
            continue;
        }
        let issue_row = |index: usize, indented: bool| Row {
            kind: Kind::Issue,
            key: issues[index].key.clone(),
            title: issues[index].summary.clone(),
            issue: Some(index),
            count: 0,
            indented,
            section: section.clone(),
        };
        if section != BACKLOG {
            rows.extend(members.into_iter().map(|index| issue_row(index, false)));
            continue;
        }
        let parent_of = |index: usize| issues[index].epic.as_ref().map(|epic| &epic.key);
        let mut grouped = HashSet::new();
        for &index in &members {
            let Some(parent) = &issues[index].epic else {
                let is_parent = members
                    .iter()
                    .any(|&other| parent_of(other) == Some(&issues[index].key));
                if !is_parent {
                    rows.push(issue_row(index, false)); // a parent is shown as the heading of its group
                }
                continue;
            };
            if !grouped.insert(&parent.key) {
                continue;
            }
            let children: Vec<usize> = members
                .iter()
                .copied()
                .filter(|&other| parent_of(other) == Some(&parent.key))
                .collect();
            rows.push(Row {
                kind: Kind::Parent,
                key: parent.key.clone(),
                title: parent.summary.clone(),
                issue: issues.iter().position(|issue| issue.key == parent.key),
                count: children.len(),
                indented: false,
                section: section.clone(),
            });
            rows.extend(children.into_iter().map(|child| issue_row(child, true)));
        }
    }
    rows
}
