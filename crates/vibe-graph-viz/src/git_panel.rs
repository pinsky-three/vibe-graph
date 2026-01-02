//! Git controls floating panel.
//!
//! Provides a modular UI for Git operations to navigate graph changes:
//! - Repository selection (multi-repo workspaces)
//! - Branch information and switching
//! - File staging/unstaging
//! - Commit creation
//! - Commit history (log)
//! - Diff viewing

#![allow(dead_code)]

use egui::{Context, RichText, ScrollArea, Ui};

#[cfg(target_arch = "wasm32")]
use std::cell::RefCell;
#[cfg(target_arch = "wasm32")]
use std::rc::Rc;

use crate::api::{
    GitBranch, GitBranchesResponse, GitDiffResponse, GitLogEntry, GitLogResponse, GitRepoInfo,
    GitReposResponse, OperationState,
};

// =============================================================================
// Panel State
// =============================================================================

/// Active tab in the Git panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GitPanelTab {
    #[default]
    Status,
    Branches,
    Log,
    Diff,
}

impl GitPanelTab {
    fn label(&self) -> &'static str {
        match self {
            Self::Status => "Status",
            Self::Branches => "Branches",
            Self::Log => "Log",
            Self::Diff => "Diff",
        }
    }

    fn icon(&self) -> &'static str {
        match self {
            Self::Status => "📋",
            Self::Branches => "🌿",
            Self::Log => "📜",
            Self::Diff => "±",
        }
    }
}

/// Result from an async Git operation.
#[derive(Clone)]
pub enum GitOpResult {
    /// Repositories list fetched.
    ReposFetched(GitReposResponse),
    /// Branches fetched.
    BranchesFetched(GitBranchesResponse),
    /// Log fetched.
    LogFetched(GitLogResponse),
    /// Diff fetched.
    DiffFetched(GitDiffResponse),
    /// Files staged.
    Staged { count: usize },
    /// Files unstaged.
    Unstaged { count: usize },
    /// Commit created.
    Committed { id: String, message: String },
    /// Branch checked out.
    CheckedOut { branch: String },
    /// Operation failed.
    Error(String),
}

/// Shared channel for async Git operation results.
#[cfg(target_arch = "wasm32")]
pub type SharedGitResult = std::rc::Rc<std::cell::RefCell<Option<GitOpResult>>>;

/// Git panel state.
pub struct GitPanelState {
    /// Whether the panel is visible.
    pub visible: bool,
    /// Current active tab.
    pub tab: GitPanelTab,
    /// Current operation state.
    pub state: OperationState,
    /// Status message.
    pub message: String,
    /// Error message.
    pub error: Option<String>,

    // Data caches
    /// Available repositories.
    pub repos: Vec<GitRepoInfo>,
    /// Default repository name.
    pub default_repo: Option<String>,
    /// Currently selected repository.
    pub selected_repo: Option<String>,
    /// Branches for the selected repo.
    pub branches: Vec<GitBranch>,
    /// Current branch name.
    pub current_branch: Option<String>,
    /// Commit log entries.
    pub log_entries: Vec<GitLogEntry>,
    /// Current diff output.
    pub diff: Option<GitDiffResponse>,
    /// Show staged diff (vs working directory).
    pub diff_staged: bool,

    // Input fields
    /// Commit message input.
    pub commit_message: String,
    /// Files to stage (paths).
    pub stage_paths: Vec<String>,

    /// Result channel for async operations.
    #[cfg(target_arch = "wasm32")]
    result_channel: SharedGitResult,
}

impl Default for GitPanelState {
    fn default() -> Self {
        Self {
            visible: false,
            tab: GitPanelTab::Status,
            state: OperationState::Idle,
            message: String::new(),
            error: None,
            repos: Vec::new(),
            default_repo: None,
            selected_repo: None,
            branches: Vec::new(),
            current_branch: None,
            log_entries: Vec::new(),
            diff: None,
            diff_staged: false,
            commit_message: String::new(),
            stage_paths: Vec::new(),
            #[cfg(target_arch = "wasm32")]
            result_channel: Rc::new(RefCell::new(None)),
        }
    }
}

impl GitPanelState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Toggle panel visibility.
    pub fn toggle(&mut self) {
        self.visible = !self.visible;
        if self.visible && self.repos.is_empty() {
            self.fetch_repos();
        }
    }

    /// Check if loading.
    pub fn is_loading(&self) -> bool {
        self.state == OperationState::Loading
    }

    /// Get effective repo (selected or default).
    pub fn effective_repo(&self) -> Option<&str> {
        self.selected_repo
            .as_deref()
            .or(self.default_repo.as_deref())
    }

    // =========================================================================
    // Async Result Polling
    // =========================================================================

    /// Poll for async operation results and update state.
    #[cfg(target_arch = "wasm32")]
    pub fn poll_results(&mut self) -> bool {
        let result = self.result_channel.borrow_mut().take();
        if let Some(res) = result {
            match res {
                GitOpResult::ReposFetched(resp) => {
                    self.repos = resp.repos;
                    self.default_repo = resp.default.clone();
                    if self.selected_repo.is_none() {
                        self.selected_repo = resp.default;
                    }
                    self.state = OperationState::Success;
                    self.message = format!("Loaded {} repositories", self.repos.len());
                    self.error = None;
                    // Auto-fetch branches after repos
                    self.fetch_branches();
                }
                GitOpResult::BranchesFetched(resp) => {
                    self.branches = resp.branches;
                    self.current_branch = resp.current;
                    self.state = OperationState::Success;
                    self.message = format!("Loaded {} branches", self.branches.len());
                    self.error = None;
                }
                GitOpResult::LogFetched(resp) => {
                    self.log_entries = resp.commits;
                    self.state = OperationState::Success;
                    self.message = format!("Loaded {} commits", self.log_entries.len());
                    self.error = None;
                }
                GitOpResult::DiffFetched(resp) => {
                    self.diff = Some(resp);
                    self.state = OperationState::Success;
                    self.message = "Diff loaded".to_string();
                    self.error = None;
                }
                GitOpResult::Staged { count } => {
                    self.state = OperationState::Success;
                    self.message = format!("Staged {} files", count);
                    self.error = None;
                    // Refresh diff after staging
                    self.fetch_diff();
                }
                GitOpResult::Unstaged { count } => {
                    self.state = OperationState::Success;
                    self.message = format!("Unstaged {} files", count);
                    self.error = None;
                    // Refresh diff after unstaging
                    self.fetch_diff();
                }
                GitOpResult::Committed { id, message } => {
                    self.state = OperationState::Success;
                    self.message = format!("Committed: {} - {}", &id[..7.min(id.len())], message);
                    self.commit_message.clear();
                    self.error = None;
                    // Refresh log after commit
                    self.fetch_log();
                }
                GitOpResult::CheckedOut { branch } => {
                    self.current_branch = Some(branch.clone());
                    self.state = OperationState::Success;
                    self.message = format!("Checked out: {}", branch);
                    self.error = None;
                    // Refresh branches after checkout
                    self.fetch_branches();
                }
                GitOpResult::Error(e) => {
                    self.state = OperationState::Error;
                    self.error = Some(e);
                }
            }
            return true;
        }
        false
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn poll_results(&mut self) -> bool {
        false
    }

    // =========================================================================
    // Git Operations
    // =========================================================================

    /// Fetch available repositories.
    #[cfg(target_arch = "wasm32")]
    pub fn fetch_repos(&mut self) {
        use crate::api::git_repos;

        self.state = OperationState::Loading;
        self.message = "Loading repositories...".to_string();

        let result_channel = self.result_channel.clone();

        wasm_bindgen_futures::spawn_local(async move {
            let result = match git_repos().await {
                Ok(resp) => GitOpResult::ReposFetched(resp),
                Err(e) => GitOpResult::Error(e),
            };
            *result_channel.borrow_mut() = Some(result);
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn fetch_repos(&mut self) {
        self.state = OperationState::Error;
        self.error = Some("Not available in native mode".to_string());
    }

    /// Fetch branches for selected repository.
    #[cfg(target_arch = "wasm32")]
    pub fn fetch_branches(&mut self) {
        use crate::api::git_branches;

        self.state = OperationState::Loading;
        self.message = "Loading branches...".to_string();

        let repo = self.effective_repo().map(|s| s.to_string());
        let result_channel = self.result_channel.clone();

        wasm_bindgen_futures::spawn_local(async move {
            let result = match git_branches(repo.as_deref()).await {
                Ok(resp) => GitOpResult::BranchesFetched(resp),
                Err(e) => GitOpResult::Error(e),
            };
            *result_channel.borrow_mut() = Some(result);
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn fetch_branches(&mut self) {
        self.state = OperationState::Error;
        self.error = Some("Not available in native mode".to_string());
    }

    /// Fetch commit log for selected repository.
    #[cfg(target_arch = "wasm32")]
    pub fn fetch_log(&mut self) {
        use crate::api::git_log;

        self.state = OperationState::Loading;
        self.message = "Loading log...".to_string();

        let repo = self.effective_repo().map(|s| s.to_string());
        let result_channel = self.result_channel.clone();

        wasm_bindgen_futures::spawn_local(async move {
            let result = match git_log(repo.as_deref(), 50).await {
                Ok(resp) => GitOpResult::LogFetched(resp),
                Err(e) => GitOpResult::Error(e),
            };
            *result_channel.borrow_mut() = Some(result);
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn fetch_log(&mut self) {
        self.state = OperationState::Error;
        self.error = Some("Not available in native mode".to_string());
    }

    /// Fetch diff for selected repository.
    #[cfg(target_arch = "wasm32")]
    pub fn fetch_diff(&mut self) {
        use crate::api::git_diff;

        self.state = OperationState::Loading;
        self.message = "Loading diff...".to_string();

        let repo = self.effective_repo().map(|s| s.to_string());
        let staged = self.diff_staged;
        let result_channel = self.result_channel.clone();

        wasm_bindgen_futures::spawn_local(async move {
            let result = match git_diff(repo.as_deref(), staged).await {
                Ok(resp) => GitOpResult::DiffFetched(resp),
                Err(e) => GitOpResult::Error(e),
            };
            *result_channel.borrow_mut() = Some(result);
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn fetch_diff(&mut self) {
        self.state = OperationState::Error;
        self.error = Some("Not available in native mode".to_string());
    }

    /// Stage all changes.
    #[cfg(target_arch = "wasm32")]
    pub fn stage_all(&mut self) {
        use crate::api::git_add;

        self.state = OperationState::Loading;
        self.message = "Staging all changes...".to_string();

        let repo = self.effective_repo().map(|s| s.to_string());
        let result_channel = self.result_channel.clone();

        wasm_bindgen_futures::spawn_local(async move {
            let result = match git_add(repo, vec![]).await {
                Ok(resp) => GitOpResult::Staged { count: resp.count },
                Err(e) => GitOpResult::Error(e),
            };
            *result_channel.borrow_mut() = Some(result);
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn stage_all(&mut self) {
        self.state = OperationState::Error;
        self.error = Some("Not available in native mode".to_string());
    }

    /// Unstage all changes.
    #[cfg(target_arch = "wasm32")]
    pub fn unstage_all(&mut self) {
        use crate::api::git_reset;

        self.state = OperationState::Loading;
        self.message = "Unstaging all changes...".to_string();

        let repo = self.effective_repo().map(|s| s.to_string());
        let result_channel = self.result_channel.clone();

        wasm_bindgen_futures::spawn_local(async move {
            let result = match git_reset(repo, vec![]).await {
                Ok(resp) => GitOpResult::Unstaged { count: resp.count },
                Err(e) => GitOpResult::Error(e),
            };
            *result_channel.borrow_mut() = Some(result);
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn unstage_all(&mut self) {
        self.state = OperationState::Error;
        self.error = Some("Not available in native mode".to_string());
    }

    /// Create a commit.
    #[cfg(target_arch = "wasm32")]
    pub fn commit(&mut self) {
        use crate::api::git_commit;

        if self.commit_message.trim().is_empty() {
            self.state = OperationState::Error;
            self.error = Some("Commit message required".to_string());
            return;
        }

        self.state = OperationState::Loading;
        self.message = "Creating commit...".to_string();

        let repo = self.effective_repo().map(|s| s.to_string());
        let message = self.commit_message.clone();
        let result_channel = self.result_channel.clone();

        wasm_bindgen_futures::spawn_local(async move {
            let result = match git_commit(repo, &message).await {
                Ok(resp) => GitOpResult::Committed {
                    id: resp.commit_id,
                    message: resp.message,
                },
                Err(e) => GitOpResult::Error(e),
            };
            *result_channel.borrow_mut() = Some(result);
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn commit(&mut self) {
        self.state = OperationState::Error;
        self.error = Some("Not available in native mode".to_string());
    }

    /// Checkout a branch.
    #[cfg(target_arch = "wasm32")]
    pub fn checkout(&mut self, branch: &str) {
        use crate::api::git_checkout;

        self.state = OperationState::Loading;
        self.message = format!("Checking out {}...", branch);

        let repo = self.effective_repo().map(|s| s.to_string());
        let branch_name = branch.to_string();
        let result_channel = self.result_channel.clone();

        wasm_bindgen_futures::spawn_local(async move {
            let result = match git_checkout(repo, &branch_name).await {
                Ok(()) => GitOpResult::CheckedOut {
                    branch: branch_name,
                },
                Err(e) => GitOpResult::Error(e),
            };
            *result_channel.borrow_mut() = Some(result);
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn checkout(&mut self, _branch: &str) {
        self.state = OperationState::Error;
        self.error = Some("Not available in native mode".to_string());
    }
}

// =============================================================================
// UI Rendering
// =============================================================================

impl GitPanelState {
    /// Render the Git panel as a floating window.
    pub fn show(&mut self, ctx: &Context) {
        // Poll for async results
        self.poll_results();

        // Request repaint while loading
        if self.is_loading() {
            ctx.request_repaint();
        }

        if !self.visible {
            return;
        }

        egui::Window::new("🔧 Git Tools")
            .id(egui::Id::new("git_panel"))
            .default_pos([100.0, 100.0])
            .default_size([400.0, 500.0])
            .resizable(true)
            .collapsible(true)
            .show(ctx, |ui| {
                self.render_header(ui);
                ui.separator();
                self.render_tabs(ui);
                ui.separator();
                self.render_content(ui);
                ui.separator();
                self.render_status_bar(ui);
            });
    }

    fn render_header(&mut self, ui: &mut Ui) {
        // Collect repo selection outside of borrow
        let mut new_repo_selected: Option<String> = None;
        let mut refresh_clicked = false;

        ui.horizontal(|ui| {
            // Repository selector
            ui.label("Repo:");
            let selected_text = self
                .selected_repo
                .as_deref()
                .or(self.default_repo.as_deref())
                .unwrap_or("(none)");

            // Clone repos for iteration to avoid borrow issues
            let repos: Vec<_> = self.repos.iter().map(|r| r.name.clone()).collect();

            egui::ComboBox::from_id_salt("repo_selector")
                .selected_text(selected_text)
                .show_ui(ui, |ui| {
                    for repo_name in &repos {
                        let is_selected = self.selected_repo.as_ref() == Some(repo_name);
                        if ui.selectable_label(is_selected, repo_name).clicked() {
                            new_repo_selected = Some(repo_name.clone());
                        }
                    }
                });

            // Current branch display
            if let Some(branch) = &self.current_branch {
                ui.separator();
                ui.label(
                    RichText::new(format!("🌿 {}", branch))
                        .color(egui::Color32::from_rgb(0, 255, 136)),
                );
            }

            // Refresh button
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let enabled = !self.is_loading();
                if ui
                    .add_enabled(enabled, egui::Button::new("🔄"))
                    .on_hover_text("Refresh")
                    .clicked()
                {
                    refresh_clicked = true;
                }
            });
        });

        // Handle deferred actions
        if let Some(repo) = new_repo_selected {
            self.selected_repo = Some(repo);
            self.fetch_branches();
            self.fetch_diff();
        }

        if refresh_clicked {
            self.refresh_current_tab();
        }
    }

    fn render_tabs(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            for tab in [
                GitPanelTab::Status,
                GitPanelTab::Branches,
                GitPanelTab::Log,
                GitPanelTab::Diff,
            ] {
                let is_selected = self.tab == tab;
                let text = format!("{} {}", tab.icon(), tab.label());
                if ui.selectable_label(is_selected, text).clicked() {
                    self.tab = tab;
                    self.refresh_current_tab();
                }
            }
        });
    }

    fn render_content(&mut self, ui: &mut Ui) {
        match self.tab {
            GitPanelTab::Status => self.render_status_tab(ui),
            GitPanelTab::Branches => self.render_branches_tab(ui),
            GitPanelTab::Log => self.render_log_tab(ui),
            GitPanelTab::Diff => self.render_diff_tab(ui),
        }
    }

    fn render_status_tab(&mut self, ui: &mut Ui) {
        // Stage/Unstage actions
        ui.horizontal(|ui| {
            let enabled = !self.is_loading();

            if ui
                .add_enabled(enabled, egui::Button::new("➕ Stage All"))
                .on_hover_text("Stage all changes (git add -A)")
                .clicked()
            {
                self.stage_all();
            }

            if ui
                .add_enabled(enabled, egui::Button::new("➖ Unstage All"))
                .on_hover_text("Unstage all changes (git reset)")
                .clicked()
            {
                self.unstage_all();
            }
        });

        ui.separator();

        // Commit section
        ui.label(RichText::new("Commit").strong());

        ui.horizontal(|ui| {
            ui.label("Message:");
            let te = egui::TextEdit::singleline(&mut self.commit_message)
                .hint_text("Enter commit message...")
                .desired_width(250.0);
            ui.add(te);
        });

        let can_commit = !self.is_loading() && !self.commit_message.trim().is_empty();
        if ui
            .add_enabled(can_commit, egui::Button::new("✓ Commit"))
            .on_hover_text("Create commit")
            .clicked()
        {
            self.commit();
        }

        ui.separator();

        // Diff summary
        if let Some(diff) = &self.diff {
            ui.label(RichText::new("Changes").strong());
            ui.horizontal(|ui| {
                ui.label(format!("Files: {}", diff.files_changed));
                ui.label(
                    RichText::new(format!("+{}", diff.insertions))
                        .color(egui::Color32::from_rgb(0, 255, 136)),
                );
                ui.label(
                    RichText::new(format!("-{}", diff.deletions))
                        .color(egui::Color32::from_rgb(255, 68, 102)),
                );
            });
        } else {
            ui.label(
                RichText::new("No diff loaded")
                    .small()
                    .color(egui::Color32::GRAY),
            );
        }
    }

    fn render_branches_tab(&mut self, ui: &mut Ui) {
        if self.branches.is_empty() {
            ui.label(
                RichText::new("No branches loaded")
                    .small()
                    .color(egui::Color32::GRAY),
            );
            return;
        }

        // Clone branches data to avoid borrow issues
        let branches: Vec<_> = self.branches.clone();
        let is_loading = self.is_loading();

        // Track branch to checkout
        let mut checkout_branch: Option<String> = None;

        ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
            // Local branches
            ui.label(RichText::new("Local").strong());
            for branch in branches.iter().filter(|b| !b.is_remote) {
                if let Some(name) = Self::render_branch_item_static(ui, branch, is_loading) {
                    checkout_branch = Some(name);
                }
            }

            // Remote branches
            let remotes: Vec<_> = branches.iter().filter(|b| b.is_remote).collect();
            if !remotes.is_empty() {
                ui.separator();
                ui.label(RichText::new("Remote").strong());
                for branch in remotes {
                    if let Some(name) = Self::render_branch_item_static(ui, branch, is_loading) {
                        checkout_branch = Some(name);
                    }
                }
            }
        });

        // Handle deferred checkout
        if let Some(branch) = checkout_branch {
            self.checkout(&branch);
        }
    }

    /// Render a branch item (static version to avoid borrow issues).
    /// Returns Some(branch_name) if checkout was clicked.
    fn render_branch_item_static(
        ui: &mut Ui,
        branch: &GitBranch,
        is_loading: bool,
    ) -> Option<String> {
        let mut checkout_clicked = None;

        ui.horizontal(|ui| {
            let is_current = branch.is_current;
            let icon = if is_current { "●" } else { "○" };
            let color = if is_current {
                egui::Color32::from_rgb(0, 255, 136)
            } else if branch.is_remote {
                egui::Color32::from_rgb(120, 140, 160)
            } else {
                egui::Color32::from_rgb(200, 200, 220)
            };

            ui.label(RichText::new(icon).color(color));
            ui.label(RichText::new(&branch.name).color(color));

            // Checkout button (only for non-current branches)
            if !is_current && !branch.is_remote {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_enabled(!is_loading, egui::Button::new("↩").small())
                        .on_hover_text("Checkout this branch")
                        .clicked()
                    {
                        checkout_clicked = Some(branch.name.clone());
                    }
                });
            }
        });

        checkout_clicked
    }

    fn render_log_tab(&mut self, ui: &mut Ui) {
        if self.log_entries.is_empty() {
            if self.state == OperationState::Idle {
                // Auto-fetch log when tab is shown
                self.fetch_log();
            }
            ui.label(
                RichText::new("Loading commits...")
                    .small()
                    .color(egui::Color32::GRAY),
            );
            return;
        }

        ScrollArea::vertical().max_height(350.0).show(ui, |ui| {
            for entry in &self.log_entries {
                self.render_log_entry(ui, entry);
            }
        });
    }

    fn render_log_entry(&self, ui: &mut Ui, entry: &GitLogEntry) {
        ui.horizontal(|ui| {
            // Short SHA
            ui.label(
                RichText::new(&entry.short_id)
                    .monospace()
                    .color(egui::Color32::from_rgb(255, 170, 0)),
            );

            // Message (truncated)
            let msg = if entry.message.len() > 50 {
                format!("{}...", &entry.message[..47])
            } else {
                entry.message.clone()
            };
            ui.label(msg);
        });

        // Author and time on second line
        ui.horizontal(|ui| {
            ui.add_space(60.0); // Indent
            ui.label(
                RichText::new(&entry.author)
                    .small()
                    .color(egui::Color32::GRAY),
            );

            // Relative time (using current time estimation)
            let age = get_current_timestamp() - entry.timestamp;
            let age_str = format_relative_time(age);
            ui.label(
                RichText::new(age_str)
                    .small()
                    .color(egui::Color32::from_rgb(100, 100, 120)),
            );
        });

        ui.add_space(4.0);
    }

    fn render_diff_tab(&mut self, ui: &mut Ui) {
        // Toggle: staged vs working directory
        ui.horizontal(|ui| {
            ui.label("Show:");
            if ui.selectable_label(!self.diff_staged, "Working").clicked() {
                self.diff_staged = false;
                self.fetch_diff();
            }
            if ui.selectable_label(self.diff_staged, "Staged").clicked() {
                self.diff_staged = true;
                self.fetch_diff();
            }
        });

        ui.separator();

        if let Some(diff) = &self.diff {
            if diff.diff.is_empty() {
                ui.label(
                    RichText::new("No changes")
                        .small()
                        .color(egui::Color32::GRAY),
                );
            } else {
                // Stats
                ui.horizontal(|ui| {
                    ui.label(format!("{} files", diff.files_changed));
                    ui.label(
                        RichText::new(format!("+{}", diff.insertions))
                            .color(egui::Color32::from_rgb(0, 255, 136)),
                    );
                    ui.label(
                        RichText::new(format!("-{}", diff.deletions))
                            .color(egui::Color32::from_rgb(255, 68, 102)),
                    );
                });

                ui.separator();

                // Diff content with syntax highlighting
                ScrollArea::both().max_height(300.0).show(ui, |ui| {
                    self.render_diff_content(ui, &diff.diff);
                });
            }
        } else {
            ui.label(
                RichText::new("No diff loaded")
                    .small()
                    .color(egui::Color32::GRAY),
            );
        }
    }

    fn render_diff_content(&self, ui: &mut Ui, diff: &str) {
        for line in diff.lines() {
            let (color, prefix) = if line.starts_with('+') && !line.starts_with("+++") {
                (egui::Color32::from_rgb(0, 255, 136), "+")
            } else if line.starts_with('-') && !line.starts_with("---") {
                (egui::Color32::from_rgb(255, 68, 102), "-")
            } else if line.starts_with("@@") {
                (egui::Color32::from_rgb(0, 212, 255), "@")
            } else if line.starts_with("diff ") || line.starts_with("index ") {
                (egui::Color32::from_rgb(255, 170, 0), "")
            } else {
                (egui::Color32::from_rgb(160, 160, 180), " ")
            };

            let display_line = if prefix.is_empty() { line } else { line };

            ui.label(RichText::new(display_line).monospace().color(color));
        }
    }

    fn render_status_bar(&self, ui: &mut Ui) {
        ui.horizontal(|ui| match self.state {
            OperationState::Loading => {
                ui.spinner();
                ui.label(
                    RichText::new(&self.message)
                        .small()
                        .color(egui::Color32::from_rgb(0, 212, 255)),
                );
            }
            OperationState::Success => {
                ui.label(
                    RichText::new(&self.message)
                        .small()
                        .color(egui::Color32::from_rgb(0, 255, 136)),
                );
            }
            OperationState::Error => {
                if let Some(err) = &self.error {
                    ui.label(
                        RichText::new(format!("❌ {}", err))
                            .small()
                            .color(egui::Color32::from_rgb(255, 68, 102)),
                    );
                }
            }
            OperationState::Idle => {
                ui.label(RichText::new("Ready").small().color(egui::Color32::GRAY));
            }
        });
    }

    fn refresh_current_tab(&mut self) {
        match self.tab {
            GitPanelTab::Status => self.fetch_diff(),
            GitPanelTab::Branches => self.fetch_branches(),
            GitPanelTab::Log => self.fetch_log(),
            GitPanelTab::Diff => self.fetch_diff(),
        }
    }
}

// =============================================================================
// Helpers
// =============================================================================

/// Get current Unix timestamp.
#[cfg(target_arch = "wasm32")]
fn get_current_timestamp() -> i64 {
    (js_sys::Date::now() / 1000.0) as i64
}

#[cfg(not(target_arch = "wasm32"))]
fn get_current_timestamp() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Format a timestamp diff as relative time.
fn format_relative_time(seconds: i64) -> String {
    if seconds < 60 {
        "just now".to_string()
    } else if seconds < 3600 {
        format!("{}m ago", seconds / 60)
    } else if seconds < 86400 {
        format!("{}h ago", seconds / 3600)
    } else if seconds < 604800 {
        format!("{}d ago", seconds / 86400)
    } else if seconds < 2592000 {
        format!("{}w ago", seconds / 604800)
    } else {
        format!("{}mo ago", seconds / 2592000)
    }
}
