use std::collections::HashMap;

use crate::apis::github::PrInfo;
use crate::apis::jira::Issue;
use crate::repos::RepoEntry;

/// A unified view of a Jira issue enriched with GitHub PR data, repo matches,
/// and active branch state. This is the primary data type the UI renders.
#[derive(Debug, Clone)]
pub struct Ticket {
    pub issue: Issue,
    pub pr: Option<PrInfo>,
    pub repos: Vec<RepoEntry>,
    pub active_branch: Option<String>,
}

impl Ticket {
    #[allow(dead_code)]
    pub fn key(&self) -> &str {
        &self.issue.key
    }
}

/// Holds all known tickets keyed by issue key.
#[derive(Debug, Clone, Default)]
pub struct TicketStore {
    tickets: HashMap<String, Ticket>,
}

/// Sources used to build a TicketStore from the current app state.
pub struct TicketSources<'a> {
    pub issues: &'a [Issue],
    pub story_children: &'a HashMap<String, Vec<Issue>>,
    pub github_prs: &'a HashMap<String, PrInfo>,
    pub active_branches: &'a HashMap<String, String>,
    pub repo_entries: &'a [RepoEntry],
}

impl TicketStore {
    pub fn from_sources(sources: &TicketSources) -> Self {
        let mut tickets = HashMap::new();

        let mut insert_issue = |issue: &Issue| {
            let key = issue.key.clone();
            if tickets.contains_key(&key) {
                return;
            }
            let pr = sources.github_prs.get(&key).cloned();
            let repos = repo_matches_for_issue(sources.repo_entries, issue);
            let active_branch = sources.active_branches.get(&key).cloned();
            tickets.insert(
                key,
                Ticket {
                    issue: issue.clone(),
                    pr,
                    repos,
                    active_branch,
                },
            );
        };

        for issue in sources.issues {
            insert_issue(issue);
        }

        for children in sources.story_children.values() {
            for child in children {
                insert_issue(child);
            }
        }

        // Insert parent references that aren't already known as full issues.
        // These are partial issues extracted from child `parent` fields.
        let all_issues = sources
            .issues
            .iter()
            .chain(sources.story_children.values().flatten());
        for issue in all_issues {
            if let Some(parent) = issue.parent() {
                if !tickets.contains_key(&parent.key) {
                    tickets.insert(
                        parent.key.clone(),
                        Ticket {
                            issue: parent,
                            pr: None,
                            repos: vec![],
                            active_branch: None,
                        },
                    );
                }
            }
        }

        Self { tickets }
    }

    pub fn get(&self, key: &str) -> Option<&Ticket> {
        self.tickets.get(key)
    }
}

fn repo_matches_for_issue(repo_entries: &[RepoEntry], issue: &Issue) -> Vec<RepoEntry> {
    if repo_entries.is_empty() {
        return Vec::new();
    }
    let labels = issue.labels();
    if labels.is_empty() {
        return Vec::new();
    }
    let normalized: std::collections::HashSet<String> = labels
        .iter()
        .map(|label| crate::repos::normalize_label(label))
        .collect();
    repo_entries
        .iter()
        .filter(|entry| normalized.contains(&entry.normalized))
        .cloned()
        .collect()
}
