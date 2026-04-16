use crate::github::{
    CheckRun, CheckStatus, CheckStep, MergeableState, PrComment, PrInfo, ReviewComment,
    ReviewThread,
};

pub(crate) fn test_pr() -> PrInfo {
    PrInfo {
        number: 42,
        title: "Snapshot coverage for UI".to_string(),
        state: "OPEN".to_string(),
        is_draft: false,
        checks: CheckStatus::Pending,
        check_runs: vec![
            CheckRun {
                name: "lint".to_string(),
                status: CheckStatus::Pass,
                started_at: Some("2024-01-01T10:00:00Z".to_string()),
                completed_at: Some("2024-01-01T10:02:00Z".to_string()),
                details_url: String::new(),
                summary: String::new(),
                text: String::new(),
                steps: vec![CheckStep {
                    name: "cargo fmt".to_string(),
                    status: CheckStatus::Pass,
                    started_at: Some("2024-01-01T10:00:00Z".to_string()),
                    completed_at: Some("2024-01-01T10:00:20Z".to_string()),
                }],
            },
            CheckRun {
                name: "build".to_string(),
                status: CheckStatus::Pending,
                started_at: None,
                completed_at: None,
                details_url: String::new(),
                summary: String::new(),
                text: String::new(),
                steps: vec![
                    CheckStep {
                        name: "compile".to_string(),
                        status: CheckStatus::Pass,
                        started_at: Some("2024-01-01T10:02:00Z".to_string()),
                        completed_at: Some("2024-01-01T10:03:00Z".to_string()),
                    },
                    CheckStep {
                        name: "integration".to_string(),
                        status: CheckStatus::Pending,
                        started_at: None,
                        completed_at: None,
                    },
                ],
            },
        ],
        url: "https://github.com/example/work-tui/pull/42".to_string(),
        head_branch: "TEST-123-snapshot-tests".to_string(),
        repo_slug: "example/work-tui".to_string(),
        comments: vec![PrComment {
            author: "riley".to_string(),
            body: "Needs more snapshots".to_string(),
            created_at: "2024-01-01T10:00:00Z".to_string(),
            updated_at: "2024-01-01T10:00:00Z".to_string(),
            url: "https://github.com/example/work-tui/pull/42#issuecomment-1".to_string(),
        }],
        review_threads: vec![ReviewThread {
            is_resolved: false,
            resolved_by: None,
            comments: vec![ReviewComment {
                author: "jordan".to_string(),
                body: "Show list layout too".to_string(),
                created_at: "2024-01-01T10:05:00Z".to_string(),
                path: Some("src/ui/sidebar.rs".to_string()),
                line: Some(22),
            }],
        }],
        changed_files: Some(3),
        additions: Some(72),
        deletions: Some(14),
        mergeable: Some(MergeableState::Unknown),
    }
}
