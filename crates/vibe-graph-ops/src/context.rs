//! OpsContext - The main service for executing operations.
//!
//! The OpsContext holds configuration and provides methods for all vibe-graph
//! operations. It can be used by CLI, REST API, or any other consumer.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use git2::{Cred, RemoteCallbacks};
use octocrab::Octocrab;
use tracing::{debug, info, warn};
use vibe_graph_core::{
    detect_references, GitChangeSnapshot, SourceCodeGraph, SourceCodeGraphBuilder,
};
use vibe_graph_git::get_git_changes;

use crate::config::Config;
use crate::error::{OpsError, OpsResult};
use crate::project::{Project, ProjectSource, Repository};
use crate::requests::*;
use crate::responses::*;
use crate::scan::scan_directory;
use crate::store::Store;
use crate::workspace::{SyncSource, WorkspaceInfo, WorkspaceKind};

/// The main operations context.
///
/// Holds configuration and provides methods for all vibe-graph operations.
/// Thread-safe and can be shared across async tasks.
#[derive(Debug, Clone)]
pub struct OpsContext {
    /// Configuration for operations.
    pub config: Config,
}

impl OpsContext {
    /// Create a new OpsContext with the given configuration.
    pub fn new(config: Config) -> Self {
        Self { config }
    }

    /// Create a new OpsContext with default configuration.
    pub fn default_config() -> OpsResult<Self> {
        Ok(Self::new(Config::load()?))
    }

    // =========================================================================
    // Sync Operations
    // =========================================================================

    /// Sync a codebase (local or remote).
    ///
    /// This is the main entry point for syncing. It detects the source type
    /// and dispatches to the appropriate handler.
    pub async fn sync(&self, request: SyncRequest) -> OpsResult<SyncResponse> {
        match &request.source {
            SyncSource::Local { path } => self.sync_local(path, &request).await,
            SyncSource::GitHubOrg { org } => self.sync_github_org(org, &request).await,
            SyncSource::GitHubRepo { owner, repo } => {
                self.sync_github_repo(owner, repo, &request).await
            }
        }
    }

    /// Sync a local path.
    async fn sync_local(&self, path: &Path, request: &SyncRequest) -> OpsResult<SyncResponse> {
        let workspace = WorkspaceInfo::detect(path)?;
        let store = Store::new(&workspace.root);

        // Check if we should use cached data
        if !request.force && store.exists() && !request.no_save {
            if let Some(project) = store.load()? {
                info!(name = %project.name, "Using cached project from .self");
                return Ok(SyncResponse {
                    project,
                    workspace: workspace.clone(),
                    path: workspace.root.clone(),
                    snapshot_created: None,
                    remote: store.load_manifest()?.and_then(|m| m.remote),
                });
            }
        }

        // Perform the sync
        let mut project = match &workspace.kind {
            WorkspaceKind::SingleRepo => self.sync_single_repo(&workspace)?,
            WorkspaceKind::MultiRepo { .. } => self.sync_multi_repo(&workspace)?,
            WorkspaceKind::PlainDirectory => self.sync_single_repo(&workspace)?,
        };

        // Expand content for small text files
        let max_size = self.config.max_content_size_kb * 1024;
        project.expand_content(|source| {
            source.size.map(|s| s < max_size).unwrap_or(false) && source.is_text()
        })?;

        // Detect git remote for single repos
        let remote = if workspace.is_single_repo() {
            detect_git_remote(&workspace.root)
        } else {
            None
        };

        // Save to .self unless --no-save
        let mut snapshot_path = None;
        if !request.no_save {
            store.save(&project, &workspace.kind, remote.clone())?;

            if request.snapshot {
                snapshot_path = Some(store.snapshot(&project)?);
            }
        }

        Ok(SyncResponse {
            project,
            workspace: workspace.clone(),
            path: workspace.root.clone(),
            snapshot_created: snapshot_path,
            remote,
        })
    }

    fn sync_single_repo(&self, workspace: &WorkspaceInfo) -> OpsResult<Project> {
        let root = &workspace.root;
        let name = &workspace.name;

        let source = ProjectSource::LocalPath { path: root.clone() };

        let mut project = Project {
            name: name.clone(),
            source,
            repositories: vec![],
        };

        let mut repo = Repository::new(name, root.display().to_string(), root.clone());
        scan_directory(&mut repo, root)?;
        project.repositories.push(repo);

        Ok(project)
    }

    fn sync_multi_repo(&self, workspace: &WorkspaceInfo) -> OpsResult<Project> {
        let mut project = Project {
            name: workspace.name.clone(),
            source: ProjectSource::LocalPaths {
                paths: workspace.repo_paths.clone(),
            },
            repositories: vec![],
        };

        for repo_path in &workspace.repo_paths {
            let repo_name = repo_path
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "repo".to_string());

            let mut repo = Repository::new(
                &repo_name,
                repo_path.display().to_string(),
                repo_path.clone(),
            );
            scan_directory(&mut repo, repo_path)?;
            project.repositories.push(repo);
        }

        Ok(project)
    }

    /// Sync a GitHub organization.
    async fn sync_github_org(&self, org: &str, request: &SyncRequest) -> OpsResult<SyncResponse> {
        self.config.validate_github()?;

        let username = self.config.github_username.clone().unwrap();
        let token = self.config.github_token.clone().unwrap();

        let octocrab = Octocrab::builder()
            .personal_token(token.clone())
            .build()
            .map_err(|e| OpsError::GitHubApiError {
                resource: org.to_string(),
                message: e.to_string(),
            })?;

        info!(org = %org, "Fetching organization repositories");

        // Fetch all repos with pagination
        let mut all_repos = Vec::new();
        let mut page = 1u32;

        loop {
            let repos = octocrab
                .orgs(org)
                .list_repos()
                .per_page(100)
                .page(page)
                .send()
                .await
                .map_err(|e| OpsError::GitHubApiError {
                    resource: format!("{}/repos", org),
                    message: e.to_string(),
                })?;

            if repos.items.is_empty() {
                break;
            }

            all_repos.extend(repos.items);
            page += 1;

            if page > 10 {
                warn!("Truncated at 1000 repositories");
                break;
            }
        }

        // Determine clone destination
        let org_dir = if request.use_cache {
            self.config.org_cache_dir(org)
        } else {
            PathBuf::from(org)
        };

        std::fs::create_dir_all(&org_dir)?;

        let mut project = Project {
            name: org.to_string(),
            source: ProjectSource::GitHubOrg {
                organization: org.to_string(),
            },
            repositories: vec![],
        };

        for repo in &all_repos {
            let repo_name = &repo.name;

            // Skip if in ignore list
            if request.ignore.iter().any(|s| s == repo_name) {
                debug!(repo = %repo_name, "Skipping ignored repository");
                continue;
            }

            let clone_url = repo
                .clone_url
                .as_ref()
                .map(|u| u.to_string())
                .unwrap_or_else(|| format!("https://github.com/{}/{}.git", org, repo_name));

            let repo_path = org_dir.join(repo_name);

            // Clone or update
            if needs_clone(&repo_path) {
                if repo_path.exists() {
                    std::fs::remove_dir_all(&repo_path)?;
                }

                if let Err(e) = clone_repository(&clone_url, &repo_path, &username, &token) {
                    warn!(repo = %repo_name, error = %e, "Failed to clone repository");
                    continue;
                }
            }

            // Scan the repository
            let mut repository = Repository::new(repo_name, &clone_url, repo_path.clone());
            scan_directory(&mut repository, &repo_path)?;
            project.repositories.push(repository);
        }

        // Expand content
        let max_size = self.config.max_content_size_kb * 1024;
        project.expand_content(|source| {
            source.size.map(|s| s < max_size).unwrap_or(false) && source.is_text()
        })?;

        // Create workspace info for org
        let workspace = WorkspaceInfo {
            root: org_dir.clone(),
            kind: WorkspaceKind::MultiRepo {
                repo_count: project.repositories.len(),
            },
            repo_paths: project
                .repositories
                .iter()
                .map(|r| r.local_path.clone())
                .collect(),
            name: org.to_string(),
        };

        // Save to .self
        if !request.no_save {
            let store = Store::new(&org_dir);
            store.save(&project, &workspace.kind, None)?;
        }

        Ok(SyncResponse {
            project,
            workspace,
            path: org_dir,
            snapshot_created: None,
            remote: Some(format!("https://github.com/{}", org)),
        })
    }

    /// Sync a single GitHub repository.
    async fn sync_github_repo(
        &self,
        owner: &str,
        repo_name: &str,
        request: &SyncRequest,
    ) -> OpsResult<SyncResponse> {
        self.config.validate_github()?;

        let username = self.config.github_username.clone().unwrap();
        let token = self.config.github_token.clone().unwrap();

        // Determine clone destination
        let repo_path = if request.use_cache {
            let cache_dir = self.config.org_cache_dir(owner);
            std::fs::create_dir_all(&cache_dir)?;
            cache_dir.join(repo_name)
        } else {
            PathBuf::from(repo_name)
        };

        let clone_url = format!("https://github.com/{}/{}.git", owner, repo_name);

        // Clone or update
        if needs_clone(&repo_path) {
            if repo_path.exists() {
                std::fs::remove_dir_all(&repo_path)?;
            }
            clone_repository(&clone_url, &repo_path, &username, &token)?;
        }

        // Build project
        let mut project = Project {
            name: repo_name.to_string(),
            source: ProjectSource::GitHubRepo {
                owner: owner.to_string(),
                repo: repo_name.to_string(),
            },
            repositories: vec![],
        };

        let mut repository = Repository::new(repo_name, &clone_url, repo_path.clone());
        scan_directory(&mut repository, &repo_path)?;
        project.repositories.push(repository);

        // Expand content
        let max_size = self.config.max_content_size_kb * 1024;
        project.expand_content(|source| {
            source.size.map(|s| s < max_size).unwrap_or(false) && source.is_text()
        })?;

        // Create workspace info
        let workspace = WorkspaceInfo {
            root: repo_path.clone(),
            kind: WorkspaceKind::SingleRepo,
            repo_paths: vec![repo_path.clone()],
            name: repo_name.to_string(),
        };

        // Save to .self
        if !request.no_save {
            let store = Store::new(&repo_path);
            store.save(
                &project,
                &workspace.kind,
                Some(format!("https://github.com/{}/{}", owner, repo_name)),
            )?;
        }

        Ok(SyncResponse {
            project,
            workspace,
            path: repo_path,
            snapshot_created: None,
            remote: Some(format!("https://github.com/{}/{}", owner, repo_name)),
        })
    }

    // =========================================================================
    // Graph Operations
    // =========================================================================

    /// Build or load a source code graph.
    pub async fn graph(&self, request: GraphRequest) -> OpsResult<GraphResponse> {
        let path = request
            .path
            .canonicalize()
            .unwrap_or_else(|_| request.path.clone());
        let store = Store::new(&path);

        if !store.exists() {
            return Err(OpsError::StoreNotFound { path });
        }

        // Try to load cached graph first (unless force rebuild)
        if !request.force {
            if let Some(graph) = store.load_graph()? {
                return Ok(GraphResponse {
                    graph,
                    saved_path: store.self_dir().join("graph.json"),
                    output_path: request.output,
                    from_cache: true,
                });
            }
        }

        // Load project and build graph
        let project = store.load()?.ok_or(OpsError::ProjectNotFound)?;

        let graph = self.build_source_graph(&project)?;

        // Save graph
        let saved_path = store.save_graph(&graph)?;

        // Also save to custom output if specified
        if let Some(ref output_path) = request.output {
            let json = serde_json::to_string_pretty(&graph)?;
            std::fs::write(output_path, &json)?;
        }

        Ok(GraphResponse {
            graph,
            saved_path,
            output_path: request.output,
            from_cache: false,
        })
    }

    /// Build a SourceCodeGraph from a Project.
    pub fn build_source_graph(&self, project: &Project) -> OpsResult<SourceCodeGraph> {
        let mut builder = SourceCodeGraphBuilder::new()
            .with_metadata("name", &project.name)
            .with_metadata("type", "source_code_graph");

        // Track all directories
        let mut all_dirs: HashSet<PathBuf> = HashSet::new();

        // Find workspace root
        let workspace_root = find_workspace_root(&project.repositories);
        if let Some(ref root) = workspace_root {
            all_dirs.insert(root.clone());
        }

        // Collect directories and add file nodes
        for repo in &project.repositories {
            all_dirs.insert(repo.local_path.clone());

            if let Some(ref ws_root) = workspace_root {
                let mut current = repo.local_path.parent();
                while let Some(dir_path) = current {
                    if dir_path == ws_root.as_path() {
                        break;
                    }
                    all_dirs.insert(dir_path.to_path_buf());
                    current = dir_path.parent();
                }
            }

            for source in &repo.sources {
                let mut current = source.path.parent();
                while let Some(dir_path) = current {
                    all_dirs.insert(dir_path.to_path_buf());
                    if dir_path == repo.local_path || dir_path.parent().is_none() {
                        break;
                    }
                    current = dir_path.parent();
                }
            }
        }

        // Add directory nodes
        for dir_path in &all_dirs {
            builder.add_directory(dir_path);
        }

        // Add file nodes
        for repo in &project.repositories {
            for source in &repo.sources {
                builder.add_file(&source.path, &source.relative_path);
            }
        }

        // Add hierarchy edges
        for repo in &project.repositories {
            for source in &repo.sources {
                if let Some(parent_dir) = source.path.parent() {
                    builder.add_hierarchy_edge(parent_dir, &source.path);
                }
            }
        }

        // Add directory hierarchy edges
        for dir_path in &all_dirs {
            if let Some(parent_dir) = dir_path.parent() {
                if all_dirs.contains(parent_dir) || parent_dir.exists() {
                    builder.add_hierarchy_edge(parent_dir, dir_path);
                }
            }
        }

        // Detect and add reference edges
        let max_size = self.config.max_content_size_kb * 1024;

        for repo in &project.repositories {
            for source in &repo.sources {
                if !source.is_text() || source.size.map(|s| s > max_size).unwrap_or(true) {
                    continue;
                }

                let content = match &source.content {
                    Some(c) => c.clone(),
                    None => match std::fs::read_to_string(&source.path) {
                        Ok(c) => c,
                        Err(_) => continue,
                    },
                };

                // Detect inline tests and mark the node
                if let Some(node_id) = builder.get_node_id(&source.path) {
                    if has_inline_tests(&content, &source.path) {
                        builder.set_node_metadata(node_id, "has_tests", "true");
                    }
                }

                let refs = detect_references(&content, &source.path);

                for reference in refs {
                    if let Some(source_id) = builder.get_node_id(&reference.source_path) {
                        if let Some(target_id) =
                            builder.find_node_by_path_suffix(&reference.target_route)
                        {
                            if source_id != target_id {
                                builder.add_edge(source_id, target_id, reference.kind);
                            }
                        }
                    }
                }
            }
        }

        info!(
            nodes = builder.node_count(),
            edges = builder.edge_count(),
            "Built SourceCodeGraph"
        );

        Ok(builder.build())
    }

    // =========================================================================
    // Status Operations
    // =========================================================================

    /// Get workspace status.
    pub async fn status(&self, request: StatusRequest) -> OpsResult<StatusResponse> {
        let workspace = WorkspaceInfo::detect(&request.path)?;
        let store = Store::new(&workspace.root);
        let stats = store.stats()?;

        let repositories = if request.detailed && !workspace.repo_paths.is_empty() {
            workspace
                .repo_paths
                .iter()
                .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
                .collect()
        } else {
            vec![]
        };

        Ok(StatusResponse {
            workspace,
            store_exists: stats.exists,
            manifest: stats.manifest,
            snapshot_count: stats.snapshot_count,
            store_size: stats.total_size,
            repositories,
        })
    }

    // =========================================================================
    // Load Operations
    // =========================================================================

    /// Load a project from .self store.
    pub async fn load(&self, request: LoadRequest) -> OpsResult<LoadResponse> {
        let path = request
            .path
            .canonicalize()
            .unwrap_or_else(|_| request.path.clone());
        let store = Store::new(&path);

        if !store.exists() {
            return Err(OpsError::StoreNotFound { path });
        }

        let project = store.load()?.ok_or(OpsError::ProjectNotFound)?;

        let manifest = store.load_manifest()?.ok_or(OpsError::ProjectNotFound)?;

        Ok(LoadResponse { project, manifest })
    }

    // =========================================================================
    // Clean Operations
    // =========================================================================

    /// Clean the .self folder.
    pub async fn clean(&self, request: CleanRequest) -> OpsResult<CleanResponse> {
        let path = request
            .path
            .canonicalize()
            .unwrap_or_else(|_| request.path.clone());
        let store = Store::new(&path);

        let cleaned = store.exists();
        if cleaned {
            store.clean()?;
        }

        Ok(CleanResponse { path, cleaned })
    }

    // =========================================================================
    // Git Changes Operations
    // =========================================================================

    /// Get git changes for a workspace.
    pub async fn git_changes(&self, request: GitChangesRequest) -> OpsResult<GitChangesResponse> {
        let path = request
            .path
            .canonicalize()
            .unwrap_or_else(|_| request.path.clone());
        let store = Store::new(&path);

        let changes = if store.exists() {
            if let Some(project) = store.load()? {
                git_changes_from_project(&project)
            } else {
                get_single_repo_changes(&path)
            }
        } else {
            get_single_repo_changes(&path)
        };

        let change_count = changes.changes.len();

        Ok(GitChangesResponse {
            changes,
            change_count,
        })
    }
}

// =============================================================================
// Helper Functions
// =============================================================================

/// Detect git remote URL for a repository.
fn detect_git_remote(path: &Path) -> Option<String> {
    let repo = git2::Repository::open(path).ok()?;
    let remote = repo.find_remote("origin").ok()?;
    remote.url().map(|s| s.to_string())
}

/// Check if a repository needs to be cloned.
fn needs_clone(repo_path: &Path) -> bool {
    if !repo_path.exists() {
        return true;
    }
    !repo_path.join(".git").exists()
}

/// Clone a repository using git2 with authentication.
fn clone_repository(url: &str, path: &Path, username: &str, token: &str) -> OpsResult<()> {
    let mut callbacks = RemoteCallbacks::new();
    callbacks.credentials(|_url, _username_from_url, _allowed_types| {
        Cred::userpass_plaintext(username, token)
    });

    let mut fetch_options = git2::FetchOptions::new();
    fetch_options.remote_callbacks(callbacks);

    git2::build::RepoBuilder::new()
        .fetch_options(fetch_options)
        .clone(url, path)
        .map_err(|e| OpsError::CloneFailed {
            repo: url.to_string(),
            message: e.to_string(),
        })?;

    Ok(())
}

/// Find the common workspace root of all repositories.
/// Detect whether a file contains inline test code based on language-specific patterns.
fn has_inline_tests(content: &str, path: &Path) -> bool {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    match ext {
        "rs" => content.contains("#[cfg(test)]") || content.contains("#[test]"),
        "py" => {
            content.contains("def test_")
                || content.contains("class Test")
                || content.contains("unittest.TestCase")
        }
        "ts" | "tsx" | "js" | "jsx" => {
            content.contains("describe(") || content.contains("it(") || content.contains("test(")
        }
        "go" => content.contains("func Test"),
        _ => false,
    }
}

fn find_workspace_root(repositories: &[Repository]) -> Option<PathBuf> {
    if repositories.is_empty() {
        return None;
    }

    if repositories.len() == 1 {
        return Some(repositories[0].local_path.clone());
    }

    let mut common: Option<PathBuf> = None;

    for repo in repositories {
        let path = &repo.local_path;
        match &common {
            None => {
                common = path.parent().map(|p| p.to_path_buf());
            }
            Some(current_common) => {
                let mut new_common = PathBuf::new();
                let common_components: Vec<_> = current_common.components().collect();
                let path_components: Vec<_> = path.components().collect();

                for (c1, c2) in common_components.iter().zip(path_components.iter()) {
                    if c1 == c2 {
                        new_common.push(c1.as_os_str());
                    } else {
                        break;
                    }
                }

                if new_common.as_os_str().is_empty() {
                    return None;
                }
                common = Some(new_common);
            }
        }
    }

    common
}

/// Get git changes from a project (aggregates all repos).
fn git_changes_from_project(project: &Project) -> GitChangeSnapshot {
    use vibe_graph_core::GitFileChange;

    let mut all_changes: Vec<GitFileChange> = Vec::new();

    for repo in &project.repositories {
        if let Ok(snapshot) = get_git_changes(&repo.local_path) {
            for mut change in snapshot.changes {
                change.path = repo.local_path.join(&change.path);
                all_changes.push(change);
            }
        }
    }

    GitChangeSnapshot {
        changes: all_changes,
        captured_at: Some(std::time::Instant::now()),
    }
}

/// Get git changes for a single repo.
fn get_single_repo_changes(path: &Path) -> GitChangeSnapshot {
    match get_git_changes(path) {
        Ok(mut changes) => {
            // Absolutize paths
            for change in &mut changes.changes {
                change.path = path.join(&change.path);
            }
            changes
        }
        Err(_) => GitChangeSnapshot::default(),
    }
}
