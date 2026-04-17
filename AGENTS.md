# AGENTS.md

## Overview

Rust TUI app (ratatui + crossterm) for managing Jira issues with GitHub PR integration. Single binary, async via tokio. Shells out to `gh` CLI for GitHub operations and `git` for branch management.

## Commands

```sh
cargo run --bin work-tui     # Run the app (requires env vars below)
cargo test                   # Run all tests
cargo insta test             # Run tests + review snapshot changes
cargo insta review           # Interactively accept/reject snapshot diffs
cargo clippy                 # Lint
```

## Environment

```sh
JIRA_URL          # e.g. https://yourteam.atlassian.net
JIRA_EMAIL        # Jira account email
JIRA_API_TOKEN    # Atlassian API token
JIRA_JQL          # JQL filter for issue list
REPOS_DIR         # Local directory containing git repos (scanned for GitHub slugs)
```

## Architecture

```
src/
  main.rs          # Entry point, event loop, key handling
  app.rs           # App state, background message dispatch, display row model
  actions/         # Background tasks (each has spawn() -> ActionMessage via mpsc)
  apis/
    jira.rs        # Jira REST client (gouqi-based)
    github.rs      # GitHub types, PR/check models (uses `gh` CLI)
  ui/
    list.rs        # Main list view
    sidebar.rs     # Detail sidebar
    status_bar.rs  # Footer status bar
    mod.rs         # Layout, shared rendering helpers
    snapshots/     # insta snapshot files
  fixtures/        # Test-only: app builders, fake issues/PRs, render helper
  cache.rs         # Disk cache (~/.cache/work-tui/snapshot.json)
  events.rs        # Event types for activity feed
  git.rs           # Git operations (branch, push, diff)
  repos.rs         # Scans REPOS_DIR, extracts GitHub slugs from remotes
  theme.rs         # Color constants
```

### Key patterns

- **Background tasks**: Actions in `actions/` each expose `spawn()` that takes a `mpsc::UnboundedSender<ActionMessage>` and runs on tokio. Results flow back through `App::handle_bg_msg()`.
- **Display model**: `App::rebuild_display_rows()` flattens issues into `Vec<DisplayRow>` (story headers, issues, inline-new, loading, empty). All UI rendering indexes into this.
- **Fixtures module**: `src/fixtures/` is `#[cfg(test)]` only (gated in `main.rs`). Provides `test_app()`, `selected_issue_app()`, `sidebar_app()`, and `render_to_string()` for snapshot tests.

## Testing

- Snapshot tests use **insta** with ratatui's `TestBackend`. Snapshots live in `src/ui/snapshots/`.
- After changing UI rendering, run `cargo insta test` then `cargo insta review` to accept new snapshots.
- `render_to_string(width, height, closure)` in `src/fixtures/render.rs` is the test render helper.
- When adding fields to `App`, update `test_app()` in `src/fixtures/app.rs` (manual struct init, no `Default`).

## Conventions

- Actions are self-contained modules with a single `spawn()` entry point.
- Theme colors are constants on `Theme` enum in `theme.rs` — use those, not raw colors.
