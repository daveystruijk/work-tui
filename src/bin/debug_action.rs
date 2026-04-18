//! Debug tool to exercise work-tui actions from the command line.
//!
//! Usage:
//!   cargo run --bin debug-action -- <subcommand> [args...]
//!
//! Subcommands:
//!   fetch-prs <repo-slugs...>
//!   fetch-pr-detail <owner/repo> <pr-number>
//!   fetch-ci-logs <owner/repo> <pr-number>
//!   fetch-issues
//!   fetch-children <parent-key>
//!   scan-repos
//!   detect-branches <issue-key> <repo-path>
//!   approve-merge <owner/repo> <pr-number>
//!   finish <issue-key> <issue-summary> <repo-path>
//!   pick-up <issue-key> <issue-summary> <repo-path>

use std::{env, path::PathBuf};

use color_eyre::{eyre::eyre, Result};
use tokio::process::Command;
use work_tui::apis::github::{self, CheckStatus};
use work_tui::apis::jira::{JiraClient, JiraConfig};

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;

    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        print_usage();
        std::process::exit(1);
    }

    match args[1].as_str() {
        "fetch-prs" => fetch_prs(&args[2..]).await?,
        "fetch-pr-detail" => fetch_pr_detail_cmd(&args[2..]).await?,
        "fetch-ci-logs" => fetch_ci_logs(&args[2..]).await?,
        "fetch-issues" => fetch_issues().await?,
        "fetch-children" => fetch_children(&args[2..]).await?,
        "scan-repos" => scan_repos().await?,
        "detect-branches" => detect_branches(&args[2..]).await?,
        "approve-merge" => approve_merge(&args[2..]).await?,
        "finish" => finish(&args[2..]).await?,
        "pick-up" => pick_up(&args[2..]).await?,
        _ => {
            eprintln!("Unknown subcommand: {}", args[1]);
            print_usage();
            std::process::exit(1);
        }
    }

    Ok(())
}

fn jira_client() -> Result<(JiraClient, String)> {
    let config = JiraConfig::from_env()?;
    let jql = config.default_jql.clone();
    let client = JiraClient::new(&config)?;
    Ok((client, jql))
}

fn print_usage() {
    eprintln!("Usage: debug-action <subcommand> [args...]");
}

async fn fetch_prs(args: &[String]) -> Result<()> {
    if args.is_empty() {
        return Err(eyre!("fetch-prs requires at least one repo slug"));
    }

    let repo_slugs = args.to_vec();
    let (prs, errors) = github::list_all_repo_prs(&repo_slugs).await;
    for error in errors {
        println!("ERROR: {error}");
    }
    for pr in prs {
        println!("{} #{}", pr.repo_slug, pr.number);
        println!("  title: {}", pr.title);
        println!("  head_branch: {}", pr.head_branch);
        println!("  checks: {:?}", pr.checks);
        for run in pr.check_runs {
            println!("  check_run: {} ({:?})", run.name, run.status);
        }
    }
    Ok(())
}

async fn fetch_pr_detail_cmd(args: &[String]) -> Result<()> {
    let (slug, pr_number) = parse_pr_args(args)?;
    let detail = github::fetch_pr_detail(&slug, pr_number).await?;
    println!("checks: {:?}", detail.checks);
    println!("comments_count: {}", detail.comments.len());
    println!("review_threads_count: {}", detail.review_threads.len());
    println!("mergeable_state: {:?}", detail.mergeable);
    for run in detail.check_runs {
        let failed_steps = run
            .steps
            .iter()
            .filter(|step| step.status == CheckStatus::Fail)
            .map(|step| step.name.clone())
            .collect::<Vec<_>>();
        println!("run: {}", run.name);
        println!("  status: {:?}", run.status);
        println!("  details_url: {}", run.details_url);
        println!("  step_count: {}", run.steps.len());
        println!("  failed_steps: {:?}", failed_steps);
    }
    Ok(())
}

async fn fetch_ci_logs(args: &[String]) -> Result<()> {
    let (slug, pr_number) = parse_pr_args(args)?;
    let detail = github::fetch_pr_detail(&slug, pr_number).await?;
    let logs = github::fetch_failed_check_run_logs(&slug, &detail.check_runs).await?;

    for (run, log_output) in detail.check_runs.iter().zip(logs.iter()) {
        if run.status != CheckStatus::Fail {
            continue;
        }
        let failed_steps: Vec<_> = run
            .steps
            .iter()
            .filter(|step| step.status == CheckStatus::Fail)
            .map(|step| step.name.clone())
            .collect();
        let has_job_segment = run.details_url.contains("/job/");
        println!("=== {} ===", run.name);
        println!("  details_url: {}", run.details_url);
        println!("  has_job_segment: {has_job_segment}");
        println!("  failed_steps: {failed_steps:?}");
        println!("  log_output_bytes: {}", log_output.len());
        if log_output.is_empty() {
            println!("  log: (empty — no output)");
        } else {
            println!("  log:");
            println!("{}", indent_block(log_output, "    "));
        }
        println!();
    }
    Ok(())
}

async fn fetch_issues() -> Result<()> {
    let (client, jql) = jira_client()?;
    let issues = client.search(&jql).await?;
    for issue in issues {
        println!("{}", issue.key);
        println!("  summary: {}", issue.summary().unwrap_or_default());
        println!("  status: {}", issue.status().map(|status| status.name).unwrap_or_default());
        println!("  labels: {:?}", issue.labels());
    }
    Ok(())
}

async fn fetch_children(args: &[String]) -> Result<()> {
    let Some(parent_key) = args.first() else { return Err(eyre!("fetch-children requires <parent-key>")); };
    let (client, _) = jira_client()?;
    let jql = format!("parent = {parent_key} ORDER BY created DESC");
    let issues = client.search(&jql).await?;
    for issue in issues {
        println!("{} | {} | {} | {:?}", issue.key, issue.summary().unwrap_or_default(), issue.status().map(|status| status.name).unwrap_or_default(), issue.labels());
    }
    Ok(())
}

async fn scan_repos() -> Result<()> {
    let repos = work_tui::repos::scan_repos()?;
    for repo in repos {
        println!("label: {}", repo.label);
        println!("  normalized: {}", repo.normalized);
        println!("  path: {}", repo.path.display());
        println!("  github_slug: {:?}", repo.github_slug);
    }
    Ok(())
}

async fn detect_branches(args: &[String]) -> Result<()> {
    if args.len() != 2 { return Err(eyre!("detect-branches requires <issue-key> <repo-path>")); }
    let issue_key = &args[0];
    let repo_path = PathBuf::from(&args[1]);
    let branch = work_tui::git::current_branch_in(&repo_path).await?;
    println!("current_branch: {}", branch);
    println!("matches_issue_key: {}", branch.contains(issue_key));
    Ok(())
}

async fn approve_merge(args: &[String]) -> Result<()> {
    let (slug, pr_number) = parse_pr_args(args)?;
    println!("WARNING: WRITE operation: approving and enabling auto-merge");
    let approve = Command::new("gh").args(["pr", "review", &pr_number.to_string(), "--approve", "--repo", &slug]).output().await?;
    println!("approve_status: {}", approve.status);
    println!("approve_stdout:\n{}", String::from_utf8_lossy(&approve.stdout));
    println!("approve_stderr:\n{}", String::from_utf8_lossy(&approve.stderr));
    let merge = Command::new("gh").args(["pr", "merge", &pr_number.to_string(), "--auto", "--merge", "--repo", &slug]).output().await?;
    println!("merge_status: {}", merge.status);
    println!("merge_stdout:\n{}", String::from_utf8_lossy(&merge.stdout));
    println!("merge_stderr:\n{}", String::from_utf8_lossy(&merge.stderr));
    Ok(())
}

async fn finish(args: &[String]) -> Result<()> {
    if args.len() != 3 { return Err(eyre!("finish requires <issue-key> <issue-summary> <repo-path>")); }
    let issue_key = &args[0];
    let issue_summary = &args[1];
    let repo_path = PathBuf::from(&args[2]);
    let (client, _) = jira_client()?;
    println!("WARNING: WRITE operation: finish workflow");
    let branch = work_tui::git::current_branch_in(&repo_path).await?;
    println!("current_branch: {}", branch);
    let clean = work_tui::git::is_clean(&repo_path).await?;
    println!("working_tree_clean: {}", clean);
    work_tui::git::fetch_origin(&repo_path).await?;
    println!("fetch_origin: ok");
    work_tui::git::push_branch(&repo_path, &branch).await?;
    println!("push_branch: ok");
    let pr_url = github::create_pr(&repo_path, &format!("{issue_key} {issue_summary}"), "").await?;
    println!("create_pr: {}", pr_url);
    let transitions = client.get_transitions(issue_key).await?;
    println!("transitions: {:#?}", transitions);
    if let Some(review) = transitions.into_iter().find(|t| t.name.to_lowercase().contains("review")) {
        client.transition_issue(issue_key, &review.id).await?;
        println!("transition_issue: {}", review.id);
    }
    Ok(())
}

async fn pick_up(args: &[String]) -> Result<()> {
    if args.len() != 3 { return Err(eyre!("pick-up requires <issue-key> <issue-summary> <repo-path>")); }
    let issue_key = &args[0];
    let issue_summary = &args[1];
    let repo_path = PathBuf::from(&args[2]);
    let (client, _) = jira_client()?;
    println!("WARNING: WRITE operation: pick-up workflow");
    let myself = client.get_myself().await?;
    let account_id = myself.account_id.unwrap_or_default();
    println!("my_account_id: {}", account_id);
    let clean = work_tui::git::is_clean(&repo_path).await?;
    println!("working_tree_clean: {}", clean);
    work_tui::git::fetch_origin(&repo_path).await?;
    println!("fetch_origin: ok");
    let branch_setup = work_tui::git::create_branch_from_origin_main(&repo_path, issue_key, issue_summary).await?;
    println!("branch_name: {}", branch_setup.branch_name);
    println!("reused_existing: {}", branch_setup.reused_existing);
    client.assign_issue(issue_key, &account_id).await?;
    println!("assign_issue: ok");
    let transitions = client.get_transitions(issue_key).await?;
    println!("transitions: {:#?}", transitions);
    if let Some(progress) = transitions.into_iter().find(|t| t.name.to_lowercase().contains("progress")) {
        client.transition_issue(issue_key, &progress.id).await?;
        println!("transition_issue: {}", progress.id);
    }
    client.move_issue_to_active_board(issue_key).await?;
    println!("move_issue_to_active_board: ok");
    Ok(())
}

fn parse_pr_args(args: &[String]) -> Result<(String, u64)> {
    if args.len() != 2 { return Err(eyre!("requires <owner/repo> <pr-number>")); }
    Ok((args[0].clone(), args[1].parse()?))
}

fn indent_block(input: &str, prefix: &str) -> String {
    input.lines().map(|line| format!("{prefix}{line}")).collect::<Vec<_>>().join("\n")
}
