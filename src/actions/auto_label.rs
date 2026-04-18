//! **Auto Label** — adds missing repo labels to issues with matched PRs.
//!
//! For each issue that has a GitHub PR but is missing the corresponding repo
//! label, this action adds the label via the Jira API. This is best-effort;
//! failures are silently ignored.
//!
//! # Channel messages produced
//! - [`ActionMessage::Progress`] (per-issue progress)
//! - [`ActionMessage::AutoLabeled`]

use tokio::sync::mpsc;

use super::ActionMessage;
use crate::actions::Progress;
use crate::apis::jira::JiraClient;

/// A single label update: `(issue_key, new_labels_list)`.
pub type LabelUpdate = (String, Vec<String>);

/// Spawn auto-labeling for the given issues.
///
/// No-ops if `to_label` is empty.
pub fn spawn(
    tx: mpsc::UnboundedSender<ActionMessage>,
    client: JiraClient,
    to_label: Vec<LabelUpdate>,
) {
    if to_label.is_empty() {
        return;
    }

    super::spawn_action(tx, "Auto-labeling", |tx| async move {
        let total = to_label.len();
        for (i, (issue_key, new_labels)) in to_label.into_iter().enumerate() {
            let _ = tx.send(ActionMessage::Progress(Progress {
                action: "auto_label",
                message: format!("Labeling {issue_key}..."),
                current: i + 1,
                total,
            }));
            let result = client.update_labels(&issue_key, &new_labels).await;
            let _ = tx.send(ActionMessage::AutoLabeled(issue_key, result.map(|_| ())));
        }
    });
}
