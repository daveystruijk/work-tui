//! One-off script to convert CamelCase Jira labels to kebab-case.
//!
//! Reads JIRA_URL, JIRA_EMAIL, JIRA_API_TOKEN, JIRA_JQL from env (same as work-tui).
//! Dry-run by default — pass `--apply` to actually update labels.
//!
//! Usage:
//!   cargo run --bin normalize-labels            # dry-run
//!   cargo run --bin normalize-labels -- --apply # apply changes

use std::collections::HashMap;
use std::env;

use color_eyre::Result;
use futures::StreamExt;
use gouqi::{r#async::Jira, Credentials, SearchOptions};

/// Explicit overrides: label -> list of replacements (empty vec = remove).
fn overrides() -> HashMap<&'static str, Vec<&'static str>> {
    HashMap::from([
        ("ProductionLabel", vec!["production_labels"]),
        (
            "AdminPanel",
            vec!["admin-panel-frontend", "admin-panel-backend"],
        ),
        ("FlutterApp", vec!["dashboard-flutter"]),
    ])
}

/// Convert a CamelCase (or PascalCase) string to kebab-case.
/// "AdminPanelBackend" -> "admin-panel-backend"
/// "mySimpleLabel"     -> "my-simple-label"
/// "already-kebab"     -> "already-kebab"
fn camel_to_kebab(input: &str) -> String {
    let mut result = String::with_capacity(input.len() + 4);
    for (i, ch) in input.chars().enumerate() {
        if ch.is_ascii_uppercase() && i > 0 {
            let prev = input.as_bytes()[i - 1] as char;
            if prev.is_ascii_lowercase() || prev.is_ascii_digit() {
                result.push('-');
            }
        }
        result.push(ch.to_ascii_lowercase());
    }
    result
}

fn needs_conversion(label: &str) -> bool {
    // Has at least one uppercase letter after position 0 preceded by a lowercase letter
    let bytes = label.as_bytes();
    for i in 1..bytes.len() {
        let ch = bytes[i] as char;
        let prev = bytes[i - 1] as char;
        if ch.is_ascii_uppercase() && (prev.is_ascii_lowercase() || prev.is_ascii_digit()) {
            return true;
        }
    }
    false
}

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;

    let apply = env::args().any(|a| a == "--apply");

    let config = work_tui::apis::jira::JiraConfig::from_env()?;
    let host = config.jira_url.trim_end_matches('/').to_string();
    let credentials = Credentials::Basic(config.jira_email, config.jira_api_token);
    let jira = Jira::new(&host, credentials)?;
    let jql = config.jira_jql;

    println!("Searching issues with: {jql}");
    if !apply {
        println!("(dry-run — pass --apply to make changes)\n");
    }

    let options = SearchOptions::builder()
        .max_results(200)
        .fields(vec!["labels"])
        .build();

    let issues: Vec<gouqi::Issue> = jira.search().stream(&jql, &options).await?.collect().await;
    println!("Found {} issues\n", issues.len());

    let overrides = overrides();
    let mut total_updates = 0usize;

    for issue in &issues {
        let labels = issue.labels();
        let mut changed = false;
        let mut new_labels: Vec<String> = Vec::new();
        for label in &labels {
            if let Some(replacements) = overrides.get(label.as_str()) {
                changed = true;
                for r in replacements {
                    new_labels.push(r.to_string());
                }
            } else if needs_conversion(label) {
                changed = true;
                new_labels.push(camel_to_kebab(label));
            } else {
                new_labels.push(label.clone());
            }
        }
        // Deduplicate (an override might add a label that already exists)
        new_labels.dedup();

        if !changed {
            continue;
        }

        println!("{}", issue.key);
        for old in &labels {
            if let Some(replacements) = overrides.get(old.as_str()) {
                if replacements.is_empty() {
                    println!("  {old} -> (removed)");
                } else {
                    println!("  {old} -> {}", replacements.join(", "));
                }
            } else if needs_conversion(old) {
                println!("  {old} -> {}", camel_to_kebab(old));
            }
        }

        if apply {
            let values: Vec<serde_json::Value> = new_labels
                .iter()
                .cloned()
                .map(serde_json::Value::String)
                .collect();
            let mut fields = std::collections::BTreeMap::new();
            fields.insert("labels".to_string(), serde_json::Value::Array(values));
            let edit = gouqi::issues::EditIssue { fields };
            jira.issues().update(&issue.key, edit).await?;
            println!("  ✓ updated");
        }

        total_updates += 1;
    }

    println!(
        "\n{total_updates} issue(s) {} label changes",
        if apply { "had" } else { "would have" }
    );
    Ok(())
}
