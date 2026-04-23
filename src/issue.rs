use crate::apis::jira::Issue;

/// Returns true if the issue type suggests this issue can contain child issues.
pub fn is_expandable(issue: &Issue) -> bool {
    let type_name = issue
        .issue_type()
        .map(|ty| ty.name)
        .unwrap_or_default()
        .to_lowercase();
    type_name.contains("story") || type_name.contains("epic")
}

/// Walk up the parent chain and collect each ancestor issue.
pub fn ancestors(issue: &Issue) -> Vec<Issue> {
    let mut result = Vec::new();
    let mut current = issue.parent();
    while let Some(parent) = current {
        result.push(parent.clone());
        current = parent.parent();
    }
    result
}

fn format_ticket_block(
    key: &str,
    summary: &str,
    description: &str,
    issue_type: Option<&str>,
    role: &str,
) -> String {
    let mut block = format!("<<< {role} TICKET START: {key} >>>\nKey: {key}\nSummary: {summary}");

    if let Some(issue_type) = issue_type {
        block.push_str(&format!("\nType: {issue_type}"));
    }

    block.push_str("\nDescription:\n");
    if description.is_empty() {
        block.push_str("(none)");
    } else {
        block.push_str(description);
    }
    block.push_str(&format!("\n<<< {role} TICKET END: {key} >>>"));
    block
}

fn format_ticket_block_from_issue(issue: &Issue) -> String {
    let issue_type = issue.issue_type().map(|issue_type| issue_type.name);
    format_ticket_block(
        &issue.key,
        &issue.summary().unwrap_or_default(),
        &issue.description().unwrap_or_default(),
        issue_type.as_deref(),
        "ANCESTOR",
    )
}

/// Format the primary issue and ancestor context from explicit fields.
pub fn format_ticket_context_parts(
    key: &str,
    summary: &str,
    description: &str,
    issue_type: Option<&str>,
    ancestors: &[Issue],
) -> String {
    let mut context = format_ticket_block(key, summary, description, issue_type, "PRIMARY");
    for ancestor in ancestors {
        context.push_str("\n\n");
        context.push_str(&format_ticket_block_from_issue(ancestor));
    }
    context
}
