//! Background actions that run asynchronously and report results via [`BgMsg`].
//!
//! Each action lives in its own module and exposes a `spawn()` function that
//! accepts the minimal context it needs (cloned handles, data snapshots) plus
//! an `mpsc::UnboundedSender<BgMsg>` to deliver results back to the main loop.
//!
//! Actions may send [`BgMsg::Progress`] messages at any time to update the
//! status bar with step-by-step feedback.

pub mod approve_merge;
pub mod add_label;
pub mod auto_label;
pub mod branch_diff;
pub mod create_inline_issue;
pub mod detect_active_branches;
pub mod fetch_github_prs;
pub mod finish;
pub mod initialize;
pub mod load_issue_events;
pub mod pick_up;
pub mod poll_ci_status;
pub mod refresh;

use std::fmt;

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
