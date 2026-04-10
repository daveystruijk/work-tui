# work-tui

A terminal UI for picking up work.

## Prerequisites

- Rust toolchain (`cargo`)
- [GitHub CLI](https://cli.github.com/) (`gh`) — authenticated and available on `$PATH`

## Configuration

All configuration is done through environment variables. All are **required**:

```sh
export JIRA_URL="https://yourteam.atlassian.net"
export JIRA_EMAIL="you@example.com"
export JIRA_API_TOKEN="your-api-token"
export JIRA_JQL="project = PROJ and status != Done and status != 'On development' and status != 'Canceled' ORDER BY updated DESC"
export GITHUB_REPOS="org/app-backend,org/app-frontend"
export REPOS_DIR="/Users/daveystruijk/code"
```

## Usage

```sh
cargo run --bin work-tui
```
