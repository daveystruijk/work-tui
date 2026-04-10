# work-tui

A terminal UI for managing Jira issues with GitHub PR integration. Built with [Ratatui](https://ratatui.rs/).

## Prerequisites

- Rust toolchain (`cargo`)
- [GitHub CLI](https://cli.github.com/) (`gh`) — authenticated and available on `$PATH`

## Configuration

All configuration is done through environment variables. All are **required**.

| Variable | Description | Example |
|---|---|---|
| `JIRA_URL` | Base URL of your Jira instance | `https://yourteam.atlassian.net` |
| `JIRA_EMAIL` | Email address for Jira authentication | `you@example.com` |
| `JIRA_API_TOKEN` | Jira API token ([create one here](https://id.atlassian.com/manage-profile/security/api-tokens)) | `ABCdef123...` |
| `JIRA_JQL` | Default JQL query to load issues | `project = PROJ AND sprint in openSprints()` |
| `GITHUB_REPOS` | Comma-separated list of GitHub repos (`owner/repo`) | `acme/backend,acme/frontend` |
| `REPOS_DIR` | Path to directory containing local Git repositories | `~/code/projects` |

Export them in your shell or use a tool like [direnv](https://direnv.net/):

```sh
export JIRA_URL="https://yourteam.atlassian.net"
export JIRA_EMAIL="you@example.com"
export JIRA_API_TOKEN="your-api-token"
export JIRA_JQL="project = PROJ AND sprint in openSprints()"
export GITHUB_REPOS="acme/backend,acme/frontend"
export REPOS_DIR="~/code/projects"
```

## Run

```sh
cargo run
```

Or build and run the release binary:

```sh
cargo build --release
./target/release/work-tui
```
