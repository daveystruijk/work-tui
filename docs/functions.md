src/main.rs
- init_logging() — Sets up file-based tracing to debug.log
- main() — Boots TUI app, sets up watchers, runs event loop, restores terminal
- setup_file_watcher(repos_dir) — Creates filesystem watcher and event channel for tasks.json changes
- try_setup_watcher(tx, repos_dir) — Watches openspec changes directory, signals rescan events
- run_app(terminal, app, fs_rx) — Main event loop: renders, ticks, handles messages, polls input
- handle_key_event(app, key_event) — Routes key presses to active UI mode or quits on Ctrl-C
src/config.rs
- AppConfig::from_env() — Loads Jira config and validates REPOS_DIR
src/theme.rs
- Theme::recency_color(elapsed_secs) — Maps age to grayscale color with quartic fadeoff
src/ticket.rs
- Ticket::key() — Returns Jira issue key
- TicketStore::from_sources(sources) — Builds ticket cache from issues, children, PRs, branches, repo matches
- TicketStore::get(key) — Looks up ticket by issue key
- repo_matches_for_issue(repo_entries, issue) — Returns repos matching issue labels
src/utils/time.rs
- parse_duration_secs(start, end) — Computes absolute seconds between two ISO timestamps
- elapsed_since_iso(ts) — Returns elapsed seconds since ISO timestamp
- parse_timestamp(ts) — Parses RFC3339 or Jira-style timestamps into UTC
- format_duration(secs) — Formats seconds as Xs, Xm, or XmYYs
- format_relative_time(timestamp) — Formats ISO timestamp as short relative age
- format_elapsed_short(secs) — Formats elapsed seconds as s, m, h, or d
src/actions/mod.rs
- Progress::fmt() — Renders progress message for display
- spawn_action(tx, task_id, label, action) — Wraps async action with start/finish lifecycle messages
src/actions/initialize.rs
- spawn(tx, client) — Starts initial background loading of user, projects, issues, PRs, branches
src/actions/refresh.rs
- spawn(tx, client, jql) — Refreshes issues using current Jira query
src/actions/fetch_jira_filters.rs
- spawn_project_statuses(tx, client, project_key) — Loads Jira statuses for one project
src/actions/fetch_github_prs.rs
- spawn(tx, org, head_prefix) — Fetches PRs for configured repos/org
src/actions/fetch_github_pr_detail.rs
- spawn(tx, repo_slug, pr_number) — Fetches detailed GitHub data for one PR
src/actions/fetch_check_run_steps.rs
- spawn(tx, repo_slug, pr_number, run_ids) — Loads step details for selected check runs
src/actions/fetch_children.rs
- spawn(tx, client, parent_key, base_jql) — Fetches child issues for a parent story/epic
- child_search_jql(parent_key, base_jql) — Builds JQL to query children
src/actions/fetch_ci_logs.rs
- spawn(tx, repo_slug, pr_number, run_ids) — Fetches failing CI log excerpts
src/actions/approve_merge.rs
- spawn(tx, repo_slug, pr_number) — Approves PR and enables auto-merge
src/actions/pick_up.rs
- spawn(tx, issue_key, repo_path) — Creates/updates branch to pick up an issue
src/actions/finish.rs
- spawn(tx, issue_key, repo_path) — Finishes work and opens PR flow
src/actions/branch_diff.rs
- spawn(tx, issue_key, repo_path) — Opens branch diff view for selected issue
src/actions/create_inline_issue.rs
- spawn(tx, ...) — Creates new Jira issue from inline input
src/actions/convert_to_story.rs
- spawn(tx, issue_key, ...) — Converts issue into a Story
src/actions/add_label.rs
- spawn(tx, issue_key, label) — Adds Jira label to an issue
src/actions/auto_label.rs
- spawn(tx, client, to_label) — Applies automatic label updates across issues
src/actions/tag_jira_repos.rs
- spawn(tx, client, ...) — Tags Jira issues with repository labels
src/actions/fix_ci.rs
- spawn(tx, issue_key, pr_info, ...) — Opens opencode session with CI failure context
src/actions/openspec_propose.rs
- spawn(tx, issue_key, ...) — Opens opencode session for openspec proposal
src/actions/import_tasks.rs
- openspec_changes_dir(repos_dir) — Locates openspec changes directory
- find_tasks_json(repos_dir, issue_key) — Finds tasks.json for an issue
- load_tasks(path) — Loads task entries from tasks.json
- spawn(tx, repos_dir, ...) — Imports tasks into Jira from openspec data
- plan_multi_task_import(issue_type_name) — Decides how to map multi-task imports to Jira types
- select_child_issue_type(issue_types, parent) — Chooses correct child issue type from metadata
- write_tasks_json(path, tasks) — Writes updated tasks back to disk
src/actions/scan_import_tasks.rs
- scan(repos_dir) — Scans repos for issues with pending import tasks
- extract_issue_key(dir_name) — Extracts issue key from directory name
- spawn(tx, repos_dir) — Background scan reporting pending import keys
src/actions/detect_active_branches.rs
- spawn(tx, issue_data) — Detects active branches for issues from local repo state
src/ui/mod.rs
- UiAnimationView::tick_spinner() — Advances loading spinner animation
- max_col_width(row_data, name) — Computes widest cell for a named column
- move_selected_index(selected_index, item_count, delta) — Moves selection clamped to bounds
- adjust_scroll_offset(selected_index, scroll_offset, item_count, viewport_height, margin) — Keeps selection visible
- render(app, frame) — Renders the whole application UI
- labeled_text_line(label, value, ...) — Builds formatted label/value line
- issue_field_string(issue, field) — Extracts string Jira field from issue
- issue_author(issue) — Resolves author display string
- humanize_timestamp(timestamp) — Formats timestamp into readable relative label
- wrap_text(text, width, max_lines) — Wraps text into lines with word truncation
- wrapped_line_count(text, width) — Counts wrapped lines for layout
- push_wrapped_block(lines, ...) — Appends wrapped lines into line buffer
- status_color(status) — Maps issue status text to UI color
- centered_rect(percent_x, percent_y, area) — Returns centered sub-rectangle
- issue_type_icon(issue_type) — Returns symbolic icon for Jira issue type
- text_char_width(text) — Measures display width of text
- truncate_word(word, width) — Truncates word to fit width
src/ui/list/mod.rs
- is_backlog_status(issue) — Checks if issue is in backlog-style status
- has_children_in_section(...) — Checks if parent has children in a section
- ListView::has_ticket_row(key) — Checks if ticket row is already displayed
- show_pick_up_dialog(app) — Opens pick-up confirmation dialog
- show_branch_diff_dialog(app) — Opens branch diff dialog
- spawn_approve_merge(app) — Starts approve-and-auto-merge action
- show_finish_dialog(app) — Opens finish confirmation dialog
- spawn_toggle_story_type(app) — Starts converting between issue/story types
- spawn_submit_inline_new(app) — Submits inline-created issues
- derive_project_key(app) — Infers active Jira project key
- open_label_picker(app) — Opens label picker popup
- open_jira_filter_picker(app) — Opens Jira filter picker popup
- open_ci_log_popup(app) — Opens CI log details for selected issue/PR
- spawn_ci_log_fetch(app, issue_key) — Fetches CI logs for selected PR
- open_import_tasks_popup(app) — Opens task import popup
- spawn_openspec_propose(app) — Opens openspec proposal session
- status_rank(issue) — Ranks statuses for sorting/display
- issue_created_str(issue) — Formats issue creation timestamp
- Plus many row/layout/rendering helpers
src/ui/list/row.rs
- find_issue_by_key(issues, key) — Finds issue in slice by key
- issue_row(...) — Builds row for a normal issue
- story_header_row(...) — Builds header row for story section
- section_header_row(...) — Builds section header row
- inline_new_row(...) — Builds inline-new-item row
- loading_row(spinner_tick, idx, depth) — Builds loading placeholder row
- empty_row(idx, depth) — Builds empty placeholder row
src/ui/list/columns/mod.rs
- highlight_spans(text, ...) — Splits text into styled spans with match highlighting
- search_match_indices(text, atoms, matcher) — Finds fuzzy/search match indices
src/ui/list/columns/ (issue, ci, pr, time, dev, status, repo .rs)
- Each has render(...) — Renders its respective column
- ci.rs also has pr_eta(check_durations, pr) — Estimates PR completion time
src/ui/filter_picker.rs
- FilterPickerView::filtered_projects(...) — Projects matching filter text
- FilterPickerView::sync_project_selection(...) — Keeps selection aligned with filtered list
- FilterPickerView::move_project_selection(...) / move_status_selection(...) — Navigation
- FilterPickerView::toggle_selected_status(...) / toggle_all_statuses(...) — Status toggles
- FilterPickerView::type_project_filter(...) / backspace_project_filter(...) — Filter input
- FilterPickerView::toggle_auto_tagging_for_draft_project() — Toggles auto-tagging
- FilterPickerView::adjust_project_scroll_offset(...) / adjust_status_scroll_offset(...) — Scroll
- available_statuses_for_project(...) / status_names_for_project(...) / default_status_names_for_project(...) — Status helpers
- pane_title_style(is_active) / pane_border_style(is_active) — Styling
- open(app) / apply(app) / close(app) — Lifecycle
- update_project_search(...) / switch_pane(...) / move_selection(...) — Input handling
- start_project_search(...) / stop_project_search(...) — Search mode
- ensure_draft_statuses_loaded(app) — Lazy-loads statuses
- toggle_auto_tagging(app) — Toggles auto-tagging in app state
src/ui/status_bar.rs
- StatusBarView::set_error(message) — Shows error alert
- render_running_action(action, spinner) — Formats running-task status
- render_alert(alert) — Formats status alert into spans
src/ui/confirm_dialog.rs
- ConfirmAction::title() — Returns dialog title
- ConfirmDialogView::render(frame) — Renders confirmation dialog
- labeled_line(label, value) / detail_line(text) — Dialog line builders
- centered_fixed_rect(width, height, area) — Centers fixed-size dialog
- update(app, key_event) — Handles dialog keyboard input
- confirm_action(app) — Executes confirmed action
src/ui/import_tasks.rs
- ImportTasksView::scroll_by(delta) — Scrolls import popup
- update(app, key_event) — Handles import popup input
- confirm_import_tasks(app) — Confirms and starts import
- popup_rect(area) — Computes popup geometry
src/ui/ci_logs.rs
- CiLogsView::open() — Resets/opens CI logs popup
- spawn_fix_ci(app) — Starts fix-ci action
- cycle_tab_from_app(app, delta) — Changes active log tab
- truncate_tab_label(label, max_width) — Truncates tab label
- popup_rect(area) — Computes popup geometry
- update(app, key_event) — Handles CI logs popup input
src/ui/sidebar.rs
- SidebarView::begin_pr_refresh(...) — Starts sidebar PR refresh
- render_sidebar_section_with_status(...) — Renders sidebar section with status
- section_height(lines) — Computes section display height
- comment_counts(pr) — Counts issue and review comments
- is_running_check_step(step) — Detects running CI step
- check_runs_changed(old_runs, new_runs) — Detects check run changes
- check_run_timing(...) / check_step_timing(...) — Timing computations
src/ui/label_picker.rs
- LabelPickerView::filtered_repo_entries(repo_entries) — Filters repo entries by search text
- add_label_from_picker(app) — Applies selected label
src/apis/jira.rs
- JiraConfig::from_env() — Loads Jira config from env
- JiraClient::new(config) — Creates Jira client
- IssueType::is_standard() — Checks if issue type is standard
- build_issue_search_jql(...) — Builds issue-search JQL
- quote_jql_identifier(value) / quote_jql_string(value) — JQL quoting
src/apis/github.rs
- PrInfo::apply_detail(detail) — Merges detailed PR data into summary
- parse_job_id_from_details_url(...) — Extracts job ID from GitHub Actions URL
- sanitize_log_excerpt(output) — Cleans CI log output
- extract_error_context(lines) — Pulls context lines around error
- strip_gh_log_prefix(line) / strip_gh_log_timestamp(line) — Log prefix stripping
- strip_ansi_sequences(input) — Removes ANSI escape codes
- parse_mergeable_state(...) / parse_review_decision(...) — PR state parsing
- extract_check_rollups(...) / extract_issue_comments(...) / extract_review_threads(...) — PR data extraction
- extract_author_login(author) — Extracts GitHub login
- aggregate_check_status(rollup) — Aggregates check statuses
- check_step_status(...) / check_run_status(...) — Maps check results to display status
src/cache.rs
- cache_path() — Resolves on-disk cache file location
- load() — Loads app cache from disk
- save(cache) — Persists cache to disk
src/issue.rs
- push_adf_text(node, text) — Recursively extracts text from Jira ADF nodes
- Plus issue description parsing, ancestor helpers, snapshot prompt data
src/repos.rs
- Repository-label normalization and lookup helpers for ticket matching
src/fixtures/ (app.rs, issue.rs, pr.rs, render.rs)
- test_app() / selected_issue_app() / sidebar_app() — Test app builders
- test_issue() — Canonical test Jira issue
- PR fixture helpers
- render_to_string(...) — Renders UI frame to string for snapshot testing
src/bin/normalize_labels.rs
- overrides() — Hardcoded label override mappings
- camel_to_kebab(input) — CamelCase → kebab-case conversion
- needs_conversion(label) — Checks if label needs normalization
src/bin/debug_action.rs
- jira_client() — Builds Jira client for CLI debugging
- print_usage() — Prints CLI usage
- parse_pr_args(args) — Parses repo and PR number arguments
- indent_block(input, prefix) — Indents multi-line block for display
