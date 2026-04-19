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

/// Format ancestor context for use in prompts.
pub fn format_ancestor_context(ancestors: &[Issue]) -> String {
    let mut context = String::new();
    for ancestor in ancestors {
        let summary = ancestor.summary().unwrap_or_default();
        context.push_str(&format!(
            "\n\nThis ticket is part of the following story: {summary}"
        ));
        let description = ancestor.description().unwrap_or_default();
        if !description.is_empty() {
            context.push_str(&format!("\n{description}"));
        }
    }
    context
}
