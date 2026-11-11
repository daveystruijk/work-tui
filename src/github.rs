use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::thread;

use anyhow::{Context, Result, bail};
use serde_json::{Map, Value};

use crate::model::{Check, CheckState, Pr, Review, issue_key_of};
use crate::shell::run;

/// What is fetched for each pull request, in three parts of similar cost. The search asks
/// for each part in its own request, side by side, because GitHub takes several seconds to
/// work out these fields for a page of pull requests and the parts do not wait for each other.
///
/// Two fields are deliberately absent. `mergeStateStatus` makes GitHub abort the query for
/// exceeding its resource limits. `reviewDecision` alone takes as long as everything else in
/// its part; the latest reviews say the same and cost next to nothing.
const PR_FIELD_PARTS: [&str; 3] = [
    "id number url headRefName isDraft mergeable additions deletions
     author { login }
     repository { nameWithOwner }
     autoMergeRequest { enabledAt }
     latestOpinionatedReviews(first: 20) { nodes { state } }",
    "id viewerDidAuthor
     viewerLatestReview { state }
     viewerLatestReviewRequest { id }
     reviewThreads(first: 50) { nodes { isResolved } }",
    "id commits(last: 1) { nodes { commit { statusCheckRollup { contexts(first: 50) { nodes {
         __typename
         ... on CheckRun { name status conclusion detailsUrl }
         ... on StatusContext { context state }
     } } } } } }",
];

/// The current user's login and the open pull requests whose branch starts with the project
/// key, by issue key.
pub fn pull_requests(org: &str, project: &str) -> Result<(String, HashMap<String, Pr>)> {
    let fetch = |fields: &str| {
        graphql(&format!(
            r#"{{
                viewer {{ login }}
                search(query: "type:pr state:open org:{org} head:{project}-", type: ISSUE, first: 100) {{
                    nodes {{ ... on PullRequest {{ {fields} }} }}
                }}
            }}"#
        ))
    };
    let parts: Vec<Result<Value>> = thread::scope(|scope| {
        let requests: Vec<_> = PR_FIELD_PARTS
            .iter()
            .map(|fields| scope.spawn(|| fetch(fields)))
            .collect();
        requests
            .into_iter()
            .map(|request| request.join().expect("GitHub request panicked"))
            .collect()
    });

    // Put the parts of each pull request back together by its id.
    let mut login = String::new();
    let mut merged: HashMap<String, Map<String, Value>> = HashMap::new();
    for part in parts {
        let mut part = part?;
        login = part["viewer"]["login"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        for node in part["search"]["nodes"].as_array_mut().into_iter().flatten() {
            if let (Some(id), Some(fields)) =
                (node["id"].as_str().map(str::to_owned), node.as_object_mut())
            {
                merged.entry(id).or_default().append(fields);
            }
        }
    }
    let nodes = Value::Array(merged.into_values().map(Value::Object).collect());
    Ok((login, by_issue_key(&nodes)))
}

/// The current state of the given pull requests, by issue key. Far quicker than the search
/// when there are only a few, which is what following a CI run needs.
pub fn pull_requests_by_id(ids: &[String]) -> Result<HashMap<String, Pr>> {
    let ids = serde_json::to_string(ids)?;
    let fields = PR_FIELD_PARTS.join(" ");
    let query = format!("{{ nodes(ids: {ids}) {{ ... on PullRequest {{ {fields} }} }} }}");
    Ok(by_issue_key(&graphql(&query)?["nodes"]))
}

/// Run a GraphQL query and return its data. GitHub answers 200 even when it gave up on the
/// query, so its errors are checked here rather than left to show up as missing pull requests.
fn graphql(query: &str) -> Result<Value> {
    let output = run(
        "gh",
        &["api", "graphql", "-f", &format!("query={query}")],
        None,
    )?;
    let mut response: Value =
        serde_json::from_str(&output).context("parsing the GitHub response")?;
    if let Some(error) = response["errors"].get(0) {
        let kind = error["type"].as_str().unwrap_or("error");
        bail!(
            "GitHub: {kind}: {}",
            error["message"].as_str().unwrap_or_default()
        );
    }
    Ok(response["data"].take())
}

fn by_issue_key(nodes: &Value) -> HashMap<String, Pr> {
    let mut prs = HashMap::new();
    for node in nodes
        .as_array()
        .into_iter()
        .flatten()
        .filter(|node| !node.is_null())
    {
        let pr = parse_pr(node);
        if let Some(key) = issue_key_of(&pr.branch) {
            prs.insert(key, pr);
        }
    }
    prs
}

fn parse_pr(node: &Value) -> Pr {
    let text = |value: &Value| value.as_str().unwrap_or_default().to_owned();
    let nodes = |value: &Value| value["nodes"].as_array().cloned().unwrap_or_default();
    let contexts = &node["commits"]["nodes"][0]["commit"]["statusCheckRollup"]["contexts"];
    Pr {
        id: text(&node["id"]),
        number: node["number"].as_u64().unwrap_or_default(),
        url: text(&node["url"]),
        repo_slug: text(&node["repository"]["nameWithOwner"]),
        branch: text(&node["headRefName"]),
        author: text(&node["author"]["login"]),
        is_draft: node["isDraft"].as_bool().unwrap_or_default(),
        review: {
            let reviews = nodes(&node["latestOpinionatedReviews"]);
            let has = |state: &str| {
                reviews
                    .iter()
                    .any(|review| review["state"].as_str() == Some(state))
            };
            if has("CHANGES_REQUESTED") {
                Review::ChangesRequested
            } else if has("APPROVED") {
                Review::Approved
            } else {
                Review::Awaited
            }
        },
        has_conflicts: node["mergeable"].as_str() == Some("CONFLICTING"),
        is_mine: node["viewerDidAuthor"].as_bool().unwrap_or_default(),
        awaits_my_review: node["viewerLatestReview"].is_null()
            || !node["viewerLatestReviewRequest"].is_null(),
        auto_merge: !node["autoMergeRequest"].is_null(),
        additions: node["additions"].as_u64().unwrap_or_default(),
        deletions: node["deletions"].as_u64().unwrap_or_default(),
        unresolved: nodes(&node["reviewThreads"])
            .iter()
            .filter(|thread| thread["isResolved"].as_bool() == Some(false))
            .count(),
        checks: nodes(contexts).iter().map(parse_check).collect(),
    }
}

fn parse_check(node: &Value) -> Check {
    if node["__typename"].as_str() == Some("StatusContext") {
        return Check {
            name: node["context"].as_str().unwrap_or_default().to_owned(),
            state: match node["state"].as_str() {
                Some("SUCCESS") => CheckState::Passed,
                Some("PENDING" | "EXPECTED") => CheckState::Running,
                _ => CheckState::Failed,
            },
            run_id: None,
        };
    }
    let state = match (node["status"].as_str(), node["conclusion"].as_str()) {
        (Some("COMPLETED"), Some("SUCCESS")) => CheckState::Passed,
        (Some("COMPLETED"), Some("SKIPPED" | "NEUTRAL")) => CheckState::Skipped,
        (Some("COMPLETED"), _) => CheckState::Failed,
        _ => CheckState::Running,
    };
    let run_id = node["detailsUrl"]
        .as_str()
        .and_then(|url| url.split("/actions/runs/").nth(1))
        .and_then(|rest| rest.split('/').next())
        .map(str::to_owned);
    Check {
        name: node["name"].as_str().unwrap_or_default().to_owned(),
        state,
        run_id,
    }
}

/// The GitHub organisation of the first repository in `repos_dir` that has a GitHub origin.
pub fn organisation(repos_dir: &Path, repos: &[String]) -> Option<String> {
    repos.iter().find_map(|repo| {
        let path = repos_dir.join(repo);
        let url = run("git", &["remote", "get-url", "origin"], Some(&path)).ok()?;
        let (_, rest) = url.split_once("github.com")?;
        let organisation = rest.trim_start_matches([':', '/']).split('/').next()?;
        (!organisation.is_empty()).then(|| organisation.to_owned())
    })
}

/// The output of the failed steps of the pull request's failed checks.
pub fn failed_logs(pr: &Pr) -> Result<String> {
    let runs: BTreeSet<&str> = pr
        .checks
        .iter()
        .filter(|check| check.state == CheckState::Failed)
        .filter_map(|check| check.run_id.as_deref())
        .collect();
    if runs.is_empty() {
        return Ok("No failed GitHub Actions runs on this pull request.".to_owned());
    }
    let mut logs = String::new();
    for run_id in runs {
        let args = [
            "run",
            "view",
            run_id,
            "--repo",
            &pr.repo_slug,
            "--log-failed",
        ];
        logs.push_str(&run("gh", &args, None)?);
        logs.push('\n');
    }
    Ok(logs
        .lines()
        .map(readable_log_line)
        .collect::<Vec<_>>()
        .join("\n"))
}

/// `gh` prefixes each line with `job<TAB>step<TAB>timestamp `; keep the job and the message,
/// and drop terminal escape sequences, which would corrupt the screen.
fn readable_log_line(line: &str) -> String {
    let mut parts = line.splitn(3, '\t');
    let (job, message) = match (parts.next(), parts.next(), parts.next()) {
        (Some(job), Some(_step), Some(rest)) => (
            job,
            rest.split_once(' ')
                .map_or(rest, |(_timestamp, message)| message),
        ),
        _ => ("", line),
    };
    let mut readable = if job.is_empty() {
        String::new()
    } else {
        format!("{job} │ ")
    };
    let mut chars = message.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            for escaped in chars.by_ref() {
                if escaped.is_ascii_alphabetic() {
                    break;
                }
            }
        } else if !c.is_control() {
            readable.push(c);
        }
    }
    readable
}

pub fn approve_and_merge(url: &str) -> Result<()> {
    run("gh", &["pr", "review", "--approve", url], None)?;
    enable_auto_merge(url)
}

pub fn enable_auto_merge(url: &str) -> Result<()> {
    run("gh", &["pr", "merge", "--auto", "--merge", url], None).map(drop)
}
