//! **Fetch Children** — re-fetches child issues for a Jira parent issue.
//!
//! # Channel messages produced
//! - [`Message::ActionStarted`]
//! - [`Message::ActionFinished`]
//! - [`Message::ChildrenLoaded`]

use tokio::sync::mpsc;

use super::Message;
use crate::apis::jira::JiraClient;

/// Spawn a Jira child issue fetch.
pub fn spawn(
    tx: mpsc::UnboundedSender<Message>,
    client: JiraClient,
    parent_key: String,
    base_jql: String,
) {
    super::spawn_action(
        tx,
        format!("fetch_children:{parent_key}"),
        format!("Fetching children for {parent_key}"),
        |tx| async move {
            let result = client
                .search(&child_search_jql(&parent_key, &base_jql))
                .await;
            let _ = tx.send(Message::ChildrenLoaded(parent_key, result));
        },
    );
}

fn child_search_jql(parent_key: &str, base_jql: &str) -> String {
    let base_jql = base_jql.trim();
    if base_jql.is_empty() {
        return format!("parent = {parent_key} ORDER BY created DESC");
    }

    let lowercase_jql = base_jql.to_ascii_lowercase();
    let order_by_index = lowercase_jql
        .find(" order by ")
        .or_else(|| lowercase_jql.starts_with("order by ").then_some(0));

    let Some(order_by_index) = order_by_index else {
        return format!("parent = {parent_key} AND ({base_jql})");
    };

    let filter = base_jql[..order_by_index].trim();
    let order_by = base_jql[order_by_index..].trim();
    if filter.is_empty() {
        return format!("parent = {parent_key} {order_by}");
    }

    format!("parent = {parent_key} AND ({filter}) {order_by}")
}

#[cfg(test)]
mod tests {
    use super::child_search_jql;

    #[test]
    fn appends_parent_filter_to_plain_jql() {
        assert_eq!(
            child_search_jql("TEST-1", "project = TEST AND status != Done"),
            "parent = TEST-1 AND (project = TEST AND status != Done)"
        );
    }

    #[test]
    fn preserves_order_by_clause() {
        assert_eq!(
            child_search_jql(
                "TEST-1",
                "project = TEST AND status != Done ORDER BY updated DESC"
            ),
            "parent = TEST-1 AND (project = TEST AND status != Done) ORDER BY updated DESC"
        );
    }

    #[test]
    fn falls_back_to_created_sort_without_base_jql() {
        assert_eq!(
            child_search_jql("TEST-1", "   "),
            "parent = TEST-1 ORDER BY created DESC"
        );
    }
}
