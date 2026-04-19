//! Background actions that run asynchronously and report results via [`ActionMessage`].
//!
//! Each action lives in its own module and exposes a `spawn()` function that
//! accepts the minimal context it needs (cloned handles, data snapshots) plus
//! an `mpsc::UnboundedSender<ActionMessage>` to deliver results back to the main loop.
//!
//! Actions may send [`ActionMessage::Progress`] messages at any time to update the
//! status bar with step-by-step feedback.

pub mod add_label;
pub mod approve_merge;
pub mod auto_label;
pub mod branch_diff;
pub mod convert_to_story;
pub mod create_inline_issue;
pub mod detect_active_branches;
pub mod fetch_children;
pub mod fetch_ci_logs;
pub mod fetch_github_pr_detail;
pub mod fetch_github_prs;
pub mod finish;
pub mod fix_ci;
pub mod import_tasks;
pub mod initialize;
pub mod link_jira_repos;
pub mod openspec_propose;
pub mod pick_up;
pub mod refresh;
pub mod scan_import_tasks;

use color_eyre::Result;
use std::collections::{HashMap, HashSet};
use std::fmt;
use tokio::sync::mpsc;

use crate::apis::{
    github::{PrDetail, PrInfo},
    jira::Issue,
};

/// Generic progress report sent by long-running actions.
///
/// Rendered in the status bar as `"[action] message (current/max)"`.
#[derive(Debug, Clone)]
pub struct Progress {
    /// Machine-readable action name (e.g. `"fetch_github_statuses"`).
    pub action: &'static str,
    /// Human-readable description of the current step.
    pub message: String,
    /// Current step (1-indexed).
    pub current: usize,
    /// Total number of steps (0 if indeterminate).
    pub total: usize,
}

impl fmt::Display for Progress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.total > 0 {
            write!(
                f,
                "[{}] {} ({}/{})",
                self.action, self.message, self.current, self.total
            )
        } else {
            write!(f, "[{}] {}", self.action, self.message)
        }
    }
}

#[derive(Debug, Clone)]
pub struct PickUpResult {
    pub branch: String,
    pub skipped_opencode: bool,
}

/// Messages sent from background actions back to the main event loop.
///
/// Each variant corresponds to a result produced by an action in [`crate::actions`].
pub enum ActionMessage {
    /// Current git branch resolved (from [`initialize`]).
    CurrentBranch(String),
    /// Jira user identity resolved (from [`initialize`]).
    Myself(Result<String>),
    /// Issues fetched from Jira (from [`initialize`] / [`refresh`]).
    Issues(Result<Vec<Issue>>),
    /// GitHub PRs fetched for all configured repos (from [`fetch_github_prs`]).
    /// Carries (successful PRs, per-repo error messages).
    GithubPrs(Vec<PrInfo>, Vec<String>),
    /// Detailed GitHub data fetched for one selected PR.
    GithubPrDetail(String, Result<PrDetail>),
    /// Active branches resolved (from [`detect_active_branches`]).
    ActiveBranches(HashMap<String, String>),
    /// Pick-up completed (from [`pick_up`]).
    PickedUp(Result<PickUpResult>),
    /// Branch diff opened (from [`branch_diff`]).
    BranchDiffOpened(Result<String>),
    /// PR approved and auto-merge enabled (from [`approve_merge`]).
    ApproveAutoMerged(Result<u64>),
    /// Finish completed — PR created (from [`finish`]).
    Finished(Result<String>),
    /// Inline new issue created (from [`create_inline_issue`]).
    InlineCreated(Result<String>),
    /// Labels updated for auto-labeling (from [`auto_label`]).
    AutoLabeled(String, Result<()>),
    /// Label added to an issue (from [`add_label`]).
    LabelAdded(Result<(String, String)>),
    /// Child issues loaded for a parent story (from [`fetch_children`]).
    ChildrenLoaded(String, Result<Vec<Issue>>),
    /// Issue type changed to Story (from [`convert_to_story`]).
    ConvertedToStory(String, Result<()>),
    /// Failed CI log excerpts fetched on demand (from [`fetch_ci_logs`]).
    /// Carries (issue_key, per-check-run log strings in order).
    CiLogsFetched(String, Result<Vec<String>>),
    /// Opencode session opened with CI error context (from [`fix_ci`]).
    /// Carries the branch name.
    FixCiOpened(Result<String>),
    /// Opencode session opened for openspec propose (from [`openspec_propose`]).
    /// Carries the slug.
    OpenspecProposeOpened(Result<String>),
    /// Tasks imported from tasks.json (from [`import_tasks`]).
    /// Carries (issue_key, result).
    TasksImported(String, Result<()>),
    /// Issue keys that have pending import tasks (from [`scan_import_tasks`]).
    PendingImportKeys(HashSet<String>),
    /// A background task has started. The payload is the human-readable task name.
    TaskStarted(String),
    /// A background task has finished. The payload is the human-readable task name.
    TaskFinished(String),
    /// Generic progress update from any long-running action.
    ///
    /// Rendered in the status bar with step-by-step feedback.
    Progress(Progress),
}

/// Spawn a background action with automatic [`ActionMessage::TaskStarted`] /
/// [`ActionMessage::TaskFinished`] bookkeeping.
///
/// The closure receives a clone of `tx` for sending result and progress
/// messages. `spawn_action` wraps it with the start/finish lifecycle messages.
pub fn spawn_action<F, Fut>(
    tx: mpsc::UnboundedSender<ActionMessage>,
    task_name: impl Into<String>,
    action: F,
) where
    F: FnOnce(mpsc::UnboundedSender<ActionMessage>) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send,
{
    let task_name = task_name.into();
    tokio::spawn(async move {
        let _ = tx.send(ActionMessage::TaskStarted(task_name.clone()));
        action(tx.clone()).await;
        let _ = tx.send(ActionMessage::TaskFinished(task_name));
    });
}
