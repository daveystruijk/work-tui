use std::collections::{HashMap, HashSet};

use serde_json::Value;

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

pub fn ancestors_from_sources(
    issue: &Issue,
    issues: &[Issue],
    story_children: &HashMap<String, Vec<Issue>>,
) -> Vec<Issue> {
    let mut result = Vec::new();
    let mut seen_keys = HashSet::new();
    let mut current = issue.parent();

    while let Some(parent) = current {
        if !seen_keys.insert(parent.key.clone()) {
            break;
        }

        let resolved_parent = find_issue_by_key(issues, story_children, &parent.key)
            .cloned()
            .unwrap_or_else(|| parent.clone());
        current = resolved_parent.parent();
        result.push(resolved_parent);
    }

    result
}

pub fn description(issue: &Issue) -> Option<String> {
    if let Some(description) = issue
        .fields
        .get("description")
        .and_then(description_from_value)
    {
        return Some(description);
    }

    issue
        .description()
        .filter(|description| !description.trim().is_empty())
}

fn find_issue_by_key<'a>(
    issues: &'a [Issue],
    story_children: &'a HashMap<String, Vec<Issue>>,
    key: &str,
) -> Option<&'a Issue> {
    issues.iter().find(|issue| issue.key == key).or_else(|| {
        story_children
            .values()
            .flat_map(|children| children.iter())
            .find(|issue| issue.key == key)
    })
}

fn description_from_value(value: &Value) -> Option<String> {
    match value {
        Value::String(text) if !text.trim().is_empty() => Some(text.trim().to_string()),
        Value::Object(_) => {
            let mut text = String::new();
            push_adf_text(value, &mut text);
            let text = text.trim().to_string();
            if text.is_empty() {
                return None;
            }
            Some(text)
        }
        _ => None,
    }
}

fn push_adf_text(node: &Value, text: &mut String) {
    let Some(node_type) = node.get("type").and_then(Value::as_str) else {
        return;
    };

    if node_type == "text" {
        if let Some(value) = node.get("text").and_then(Value::as_str) {
            text.push_str(value);
        }
        return;
    }

    if node_type == "hardBreak" {
        text.push('\n');
        return;
    }

    let start_len = text.len();
    let Some(children) = node.get("content").and_then(Value::as_array) else {
        return;
    };

    for child in children {
        push_adf_text(child, text);
    }

    if text.len() == start_len {
        return;
    }

    if matches!(
        node_type,
        "paragraph" | "heading" | "blockquote" | "panel" | "codeBlock" | "listItem" | "tableRow"
    ) && !text.ends_with("\n\n")
    {
        if text.ends_with('\n') {
            text.push('\n');
            return;
        }
        text.push_str("\n\n");
    }
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
        &description(issue).unwrap_or_default(),
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use serde_json::json;

    use super::{ancestors_from_sources, description};
    use crate::fixtures::test_issue;

    #[test]
    fn parses_adf_description_text() {
        let mut issue = test_issue();
        issue.fields.insert(
            "description".to_string(),
            json!({
                "type": "doc",
                "version": 1,
                "content": [
                    {
                        "type": "paragraph",
                        "content": [
                            { "type": "text", "text": "First line" },
                            { "type": "hardBreak" },
                            { "type": "text", "text": "Second line" }
                        ]
                    },
                    {
                        "type": "paragraph",
                        "content": [
                            { "type": "text", "text": "Third line" }
                        ]
                    }
                ]
            }),
        );

        assert_eq!(
            description(&issue).as_deref(),
            Some("First line\nSecond line\n\nThird line")
        );
    }

    #[test]
    fn keeps_plain_string_description() {
        let issue = test_issue();

        assert_eq!(
            description(&issue).as_deref(),
            Some("Confirm sidebar, list, and command bar layouts.")
        );
    }

    #[test]
    fn resolves_ancestors_from_loaded_issues() {
        let mut epic = test_issue();
        epic.key = "EPIC-1".to_string();
        epic.fields
            .insert("summary".to_string(), json!("Epic summary"));
        epic.fields
            .insert("description".to_string(), json!("Epic description"));

        let mut story = test_issue();
        story.key = "STORY-1".to_string();
        story
            .fields
            .insert("summary".to_string(), json!("Story summary"));
        story
            .fields
            .insert("description".to_string(), json!("Story description"));
        story.fields.insert(
            "parent".to_string(),
            json!({
                "id": epic.id.clone(),
                "key": epic.key.clone(),
                "self": epic.self_link.clone(),
                "fields": {
                    "summary": "Epic summary",
                    "issuetype": {
                        "description": "",
                        "iconUrl": "",
                        "id": "10000",
                        "name": "Epic",
                        "self": "http://localhost/issuetype/10000",
                        "subtask": false
                    }
                }
            }),
        );

        let mut child = test_issue();
        child.key = "TASK-1".to_string();
        child.fields.insert(
            "parent".to_string(),
            json!({
                "id": story.id.clone(),
                "key": story.key.clone(),
                "self": story.self_link.clone(),
                "fields": {
                    "summary": "Story summary",
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

        let ancestors =
            ancestors_from_sources(&child, &[story.clone(), epic.clone()], &HashMap::new());

        assert_eq!(
            ancestors
                .iter()
                .map(|issue| issue.key.as_str())
                .collect::<Vec<_>>(),
            vec!["STORY-1", "EPIC-1"]
        );
        assert_eq!(
            description(&ancestors[0]).as_deref(),
            Some("Story description")
        );
        assert_eq!(
            description(&ancestors[1]).as_deref(),
            Some("Epic description")
        );
    }
}
