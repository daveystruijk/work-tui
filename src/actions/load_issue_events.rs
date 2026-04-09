//! **Load Issue Events** — fetches the combined Jira + GitHub event timeline.
//!
//! Merges Jira changelog entries with GitHub PR events (reviews, CI checks)
//! into a single sorted timeline for the detail view's activity panel.
//!
//! # Channel messages produced
//! - [`BgMsg::Progress`]
//! - [`BgMsg::IssueEvents`]

use std::path::PathBuf;

use tokio::sync::mpsc;

use crate::actions::Progress;
use crate::app::BgMsg;
use crate::events::EventLoadState;
use crate::github::{self, GithubStatus};
use crate::jira::JiraClient;

/// Spawn event loading for a single issue.
///
/// `gh_pr` and `repo_path` are optional — when present, GitHub PR events are
/// included in the timeline.
pub fn spawn(
    tx: mpsc::UnboundedSender<BgMsg>,
    client: JiraClient,
    issue_key: String,
    gh_pr: Option<GithubStatus>,
    repo_path: Option<PathBuf>,
) {
    let _ = tx.send(BgMsg::Progress(Progress {
        action: "load_issue_events",
        message: format!("Loading events for {issue_key}..."),
        current: 1,
        total: 2,
    }));

    tokio::spawn(async move {
        let mut all_events = Vec::new();

        // Step 1: Jira events
        match client.get_issue_events(&issue_key).await {
            Ok(events) => all_events.extend(events),
            Err(err) => {
                let _ = tx.send(BgMsg::IssueEvents(
                    issue_key,
                    EventLoadState::Error(err.to_string()),
                ));
                return;
            }
        }

        // Step 2: GitHub PR events (if applicable)
        let _ = tx.send(BgMsg::Progress(Progress {
            action: "load_issue_events",
            message: format!("Loading GitHub events for {issue_key}..."),
            current: 2,
            total: 2,
        }));

        if let Some(GithubStatus::Found(pr)) = gh_pr {
            if let Some(path) = repo_path {
                if let Ok(events) = github::get_pr_events(&path, pr.number).await {
                    all_events.extend(events);
                }
            }
        }

        all_events.sort_by(|a, b| b.at.cmp(&a.at));
        let _ = tx.send(BgMsg::IssueEvents(
            issue_key,
            EventLoadState::Loaded(all_events),
        ));
    });
}
