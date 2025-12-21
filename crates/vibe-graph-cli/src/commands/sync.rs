//! Sync command implementation.
//!
//! The primary command that auto-detects workspace structure and builds the graph.
//! Supports multiple input sources:
//!
//! - **Local path**: scans a single git repository, multi-repo workspace, or plain directory
//! - **GitHub org**: clones all repositories from a GitHub organization
//! - **GitHub repo**: clones a single repository from GitHub
//!
//! Examples:
//! ```bash
//! vg sync                              # scan current directory
//! vg sync ./my-project                 # scan local path
//! vg sync pinsky-three                 # clone entire GitHub org
//! vg sync pinsky-three/vibe-graph      # clone single GitHub repo
//! vg sync https://github.com/org/repo  # clone from URL
//! ```

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use git2::{Cred, RemoteCallbacks};
use octocrab::Octocrab;
use tracing::{debug, info};
use walkdir::WalkDir;

use crate::config::Config;
use crate::project::{Project, ProjectSource, Repository, Source};

/// Detected workspace type based on directory structure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceKind {
    /// Single git repository (has .git in root)
    SingleRepo,
    /// Multiple repositories in subdirectories
    MultiRepo { repo_count: usize },
    /// Plain directory without git
    PlainDirectory,
}

impl std::fmt::Display for WorkspaceKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WorkspaceKind::SingleRepo => write!(f, "single repository"),
            WorkspaceKind::MultiRepo { repo_count } => {
                write!(f, "workspace with {} repositories", repo_count)
            }
            WorkspaceKind::PlainDirectory => write!(f, "plain directory"),
        }
    }
}

// =============================================================================
// Sync Source Detection
// =============================================================================

/// The source type for a sync operation.
#[derive(Debug, Clone)]
pub enum SyncSource {
    /// A local filesystem path.
    Local { path: PathBuf },
    /// A GitHub organization (clone all repos).
    GitHubOrg { org: String },
    /// A single GitHub repository.
    GitHubRepo { owner: String, repo: String },
}

impl SyncSource {
    /// Parse an input string and detect the source type.
    ///
    /// Detection rules:
    /// - Starts with `.`, `/`, or `~` → local path
    /// - Contains `/` with two segments → `owner/repo`
    /// - Single segment without path chars → org name
    /// - Full GitHub URL → extract owner/repo or org
    pub fn detect(input: &str) -> Self {
        let input = input.trim();

        // Check for explicit local path indicators
        if input.starts_with('.')
            || input.starts_with('/')
            || input.starts_with('~')
            || PathBuf::from(input).exists()
        {
            return Self::Local {
                path: PathBuf::from(input),
            };
        }

        // Clean up GitHub URLs
        let cleaned = input
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_start_matches("git@github.com:")
            .trim_start_matches("github.com/")
            .trim_end_matches('/')
            .trim_end_matches(".git");

        let parts: Vec<&str> = cleaned.split('/').filter(|s| !s.is_empty()).collect();

        match parts.len() {
            0 => Self::Local {
                path: PathBuf::from("."),
            },
            1 => {
                // Single segment: could be org name or local dir
                // If it exists locally, treat as local
                let path = PathBuf::from(input);
                if path.exists() {
                    Self::Local { path }
                } else {
                    Self::GitHubOrg {
                        org: parts[0].to_string(),
                    }
                }
            }
            2 => Self::GitHubRepo {
                owner: parts[0].to_string(),
                repo: parts[1].to_string(),
            },
            _ => {
                // More than 2 parts, might be a full URL with extra path
                // Try owner/repo from first two parts
                Self::GitHubRepo {
                    owner: parts[0].to_string(),
                    repo: parts[1].to_string(),
                }
            }
        }
    }

    /// Check if this is a remote source (requires GitHub API).
    pub fn is_remote(&self) -> bool {
        matches!(self, Self::GitHubOrg { .. } | Self::GitHubRepo { .. })
    }
}

impl std::fmt::Display for SyncSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SyncSource::Local { path } => write!(f, "local:{}", path.display()),
            SyncSource::GitHubOrg { org } => write!(f, "github-org:{}", org),
            SyncSource::GitHubRepo { owner, repo } => write!(f, "github:{}/{}", owner, repo),
        }
    }
}

/// Result of workspace detection.
#[derive(Debug)]
pub struct WorkspaceInfo {
    /// Root path of the workspace.
    pub root: PathBuf,
    /// Detected workspace kind.
    pub kind: WorkspaceKind,
    /// Paths to git repositories found.
    pub repo_paths: Vec<PathBuf>,
    /// Name derived from directory.
    pub name: String,
}

/// Detect the workspace type for a given path.
pub fn detect_workspace(path: &Path) -> Result<WorkspaceInfo> {
    let root = path
        .canonicalize()
        .with_context(|| format!("Failed to resolve path: {}", path.display()))?;

    let name = root
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "workspace".to_string());

    // Check if root is a git repo
    if root.join(".git").exists() {
        debug!(path = %root.display(), "Detected single git repository");
        return Ok(WorkspaceInfo {
            root: root.clone(),
            kind: WorkspaceKind::SingleRepo,
            repo_paths: vec![root],
            name,
        });
    }

    // Check for subdirectories that are git repos
    let mut repo_paths = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&root) {
        for entry in entries.filter_map(|e| e.ok()) {
            let entry_path = entry.path();
            if entry_path.is_dir() && entry_path.join(".git").exists() {
                repo_paths.push(entry_path);
            }
        }
    }

    if !repo_paths.is_empty() {
        // Sort for consistent ordering
        repo_paths.sort();
        let repo_count = repo_paths.len();
        debug!(
            path = %root.display(),
            repo_count,
            "Detected multi-repo workspace"
        );
        return Ok(WorkspaceInfo {
            root,
            kind: WorkspaceKind::MultiRepo { repo_count },
            repo_paths,
            name,
        });
    }

    // Plain directory
    debug!(path = %root.display(), "Detected plain directory");
    Ok(WorkspaceInfo {
        root: root.clone(),
        kind: WorkspaceKind::PlainDirectory,
        repo_paths: vec![root],
        name,
    })
}

/// Execute the sync command.
pub fn execute(config: &Config, path: &Path, verbose: bool) -> Result<Project> {
    let workspace = detect_workspace(path)?;

    println!("📁 Workspace: {}", workspace.name);
    println!("📍 Path: {}", workspace.root.display());
    println!("🔍 Detected: {}", workspace.kind);
    println!();

    let mut project = match &workspace.kind {
        WorkspaceKind::SingleRepo => {
            info!(name = %workspace.name, "Syncing single repository");
            sync_single_repo(&workspace)?
        }
        WorkspaceKind::MultiRepo { repo_count } => {
            info!(
                name = %workspace.name,
                repos = repo_count,
                "Syncing multi-repo workspace"
            );
            sync_multi_repo(&workspace)?
        }
        WorkspaceKind::PlainDirectory => {
            info!(name = %workspace.name, "Syncing plain directory");
            sync_single_repo(&workspace)?
        }
    };

    // Print summary
    println!("✅ Sync complete");
    println!("   Repositories: {}", project.repositories.len());
    println!("   Total files:  {}", project.total_sources());
    println!(
        "   Total size:   {}",
        humansize::format_size(project.total_size(), humansize::DECIMAL)
    );

    if verbose {
        println!();
        for repo in &project.repositories {
            println!("   📦 {} ({} files)", repo.name, repo.sources.len());
        }
    }

    // Optionally expand content for small files
    let max_size = config.max_content_size_kb * 1024;
    project.expand_content(|source| {
        source.size.map(|s| s < max_size).unwrap_or(false) && source.is_text()
    })?;

    Ok(project)
}

/// Sync a single repository or plain directory.
fn sync_single_repo(workspace: &WorkspaceInfo) -> Result<Project> {
    let root = &workspace.root;
    let name = &workspace.name;

    let source = match &workspace.kind {
        WorkspaceKind::SingleRepo => ProjectSource::LocalPath { path: root.clone() },
        _ => ProjectSource::LocalPath { path: root.clone() },
    };

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

/// Sync a multi-repo workspace.
fn sync_multi_repo(workspace: &WorkspaceInfo) -> Result<Project> {
    let mut project = Project {
        name: workspace.name.clone(),
        source: ProjectSource::LocalPaths {
            paths: workspace.repo_paths.clone(),
        },
        repositories: vec![],
    };

    for (i, repo_path) in workspace.repo_paths.iter().enumerate() {
        let repo_name = repo_path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| format!("repo-{}", i));

        print!(
            "   [{}/{}] Scanning {}... ",
            i + 1,
            workspace.repo_paths.len(),
            repo_name
        );

        let mut repo = Repository::new(
            &repo_name,
            repo_path.display().to_string(),
            repo_path.clone(),
        );
        scan_directory(&mut repo, repo_path)?;

        println!("{} files", repo.sources.len());
        project.repositories.push(repo);
    }

    Ok(project)
}

/// Scan a directory and populate repository sources.
fn scan_directory(repo: &mut Repository, path: &Path) -> Result<()> {
    for entry in WalkDir::new(path)
        .into_iter()
        .filter_entry(|e| !is_hidden(e) && !is_blacklisted(e))
        .filter_map(|e| e.ok())
    {
        if entry.path().is_file() {
            if let Ok(source) = Source::from_path(entry.path().to_path_buf(), &repo.local_path) {
                repo.sources.push(source);
            }
        }
    }
    Ok(())
}

/// Check if entry is hidden (starts with .).
fn is_hidden(entry: &walkdir::DirEntry) -> bool {
    entry
        .file_name()
        .to_str()
        .map(|s| s.starts_with('.'))
        .unwrap_or(false)
}

/// Check if entry is in a blacklisted directory.
fn is_blacklisted(entry: &walkdir::DirEntry) -> bool {
    const BLACKLIST: &[&str] = &[
        "node_modules",
        "target",
        "dist",
        "build",
        "__pycache__",
        ".venv",
        "venv",
        ".git",
        "vendor",
        ".next",
        "coverage",
        ".turbo",
    ];

    entry
        .file_name()
        .to_str()
        .map(|s| BLACKLIST.contains(&s))
        .unwrap_or(false)
}

// =============================================================================
// Remote Sync (GitHub)
// =============================================================================

/// Result of a remote sync operation.
#[derive(Debug)]
pub struct RemoteSyncResult {
    /// The synced project.
    pub project: Project,
    /// Path where the repo/org was cloned.
    pub cloned_path: PathBuf,
}

/// Execute sync for remote sources (GitHub org or repo).
///
/// By default, clones to the current directory. If `use_cache` is true,
/// clones to the global cache directory instead.
pub async fn execute_remote(
    config: &Config,
    source: &SyncSource,
    ignore_list: &[String],
    verbose: bool,
    use_cache: bool,
) -> Result<RemoteSyncResult> {
    match source {
        SyncSource::GitHubOrg { org } => {
            clone_github_org(config, org, ignore_list, verbose, use_cache).await
        }
        SyncSource::GitHubRepo { owner, repo } => {
            clone_github_repo(config, owner, repo, verbose, use_cache).await
        }
        SyncSource::Local { .. } => {
            anyhow::bail!("execute_remote called with local source - use execute() instead")
        }
    }
}

/// Clone a single GitHub repository.
///
/// By default clones to `./<repo_name>` in the current directory.
/// If `use_cache` is true, clones to the global cache directory.
pub async fn clone_github_repo(
    config: &Config,
    owner: &str,
    repo_name: &str,
    verbose: bool,
    use_cache: bool,
) -> Result<RemoteSyncResult> {
    config.validate_github()?;

    let username = config.github_username.clone().unwrap();
    let token = config.github_token.clone().unwrap();

    // Determine clone destination
    let repo_path = if use_cache {
        let cache_dir = config.org_cache_dir(owner);
        std::fs::create_dir_all(&cache_dir)
            .with_context(|| format!("Failed to create cache dir: {}", cache_dir.display()))?;
        cache_dir.join(repo_name)
    } else {
        // Clone to current directory
        PathBuf::from(repo_name)
    };

    let clone_url = format!("https://github.com/{}/{}.git", owner, repo_name);

    println!("📦 Repository: {}/{}", owner, repo_name);
    println!("📍 Clone to:   {}", repo_path.display());
    println!();

    // Clone or update
    if needs_clone(&repo_path) {
        println!("📥 Cloning...");

        // Clean up incomplete clone if exists
        if repo_path.exists() {
            std::fs::remove_dir_all(&repo_path)?;
        }

        clone_repository(&clone_url, &repo_path, &username, &token)?;
        println!("✅ Clone complete");
    } else {
        println!("✓ Using existing repository");
    }

    // Build project from cloned repo
    let mut project = Project {
        name: repo_name.to_string(),
        source: ProjectSource::GitHubRepo {
            owner: owner.to_string(),
            repo: repo_name.to_string(),
        },
        repositories: vec![],
    };

    // Scan the repository
    let mut repository = Repository::new(repo_name, &clone_url, repo_path.clone());
    scan_directory(&mut repository, &repo_path)?;

    println!();
    println!("✅ Sync complete");
    println!("   Files: {}", repository.sources.len());
    if verbose {
        for source in &repository.sources {
            println!("      {}", source.relative_path);
        }
    }

    project.repositories.push(repository);

    Ok(RemoteSyncResult {
        project,
        cloned_path: repo_path,
    })
}

/// Clone all repositories from a GitHub organization.
///
/// By default clones to `./<org>/` in the current directory.
/// If `use_cache` is true, clones to the global cache directory.
pub async fn clone_github_org(
    config: &Config,
    org: &str,
    ignore_list: &[String],
    verbose: bool,
    use_cache: bool,
) -> Result<RemoteSyncResult> {
    config.validate_github()?;

    let username = config.github_username.clone().unwrap();
    let token = config.github_token.clone().unwrap();

    let octocrab = Octocrab::builder().personal_token(token.clone()).build()?;

    println!("🏢 Organization: {}", org);
    println!();

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
            .with_context(|| format!("Failed to fetch repos for org: {} (page {})", org, page))?;

        if repos.items.is_empty() {
            break;
        }

        all_repos.extend(repos.items);
        page += 1;

        // Safety limit
        if page > 10 {
            println!("⚠️  Truncated at 1000 repositories");
            break;
        }
    }

    println!("📋 Found {} repositories", all_repos.len());

    let mut project = Project {
        name: org.to_string(),
        source: ProjectSource::GitHubOrg {
            organization: org.to_string(),
        },
        repositories: vec![],
    };

    // Determine clone destination
    let org_dir = if use_cache {
        config.org_cache_dir(org)
    } else {
        // Clone to ./<org>/ in current directory
        PathBuf::from(org)
    };

    std::fs::create_dir_all(&org_dir)
        .with_context(|| format!("Failed to create directory: {}", org_dir.display()))?;

    println!("📍 Clone to: {}", org_dir.display());
    println!();

    for (i, repo) in all_repos.iter().enumerate() {
        let repo_name = &repo.name;

        // Skip if in ignore list
        if ignore_list.iter().any(|s| s == repo_name) {
            println!(
                "[{}/{}] ⏭️  {} (ignored)",
                i + 1,
                all_repos.len(),
                repo_name
            );
            continue;
        }

        let clone_url = repo
            .clone_url
            .as_ref()
            .map(|u| u.to_string())
            .unwrap_or_else(|| format!("https://github.com/{}/{}.git", org, repo_name));

        let repo_path = org_dir.join(repo_name);

        // Clone or update repository
        if needs_clone(&repo_path) {
            print!(
                "[{}/{}] 📦 Cloning {}... ",
                i + 1,
                all_repos.len(),
                repo_name
            );

            // Clean up incomplete clone if exists
            if repo_path.exists() {
                std::fs::remove_dir_all(&repo_path)?;
            }

            match clone_repository(&clone_url, &repo_path, &username, &token) {
                Ok(_) => println!("✓"),
                Err(e) => {
                    println!("✗ {}", e);
                    continue; // Skip this repo but continue with others
                }
            }
        } else if verbose {
            println!("[{}/{}] ✓ {}", i + 1, all_repos.len(), repo_name);
        }

        // Scan the repository
        let mut repository = Repository::new(repo_name, &clone_url, repo_path.clone());
        scan_directory(&mut repository, &repo_path)?;
        project.repositories.push(repository);
    }

    println!();
    println!(
        "✅ Synced {} repositories ({} files total)",
        project.repositories.len(),
        project.total_sources()
    );

    Ok(RemoteSyncResult {
        project,
        cloned_path: org_dir,
    })
}

/// Check if a repository needs to be cloned.
fn needs_clone(repo_path: &Path) -> bool {
    if !repo_path.exists() {
        return true;
    }
    // Check if it's a valid git repo
    !repo_path.join(".git").exists()
}

/// Clone a repository using git2 with authentication.
fn clone_repository(url: &str, path: &Path, username: &str, token: &str) -> Result<()> {
    let mut callbacks = RemoteCallbacks::new();
    callbacks.credentials(|_url, _username_from_url, _allowed_types| {
        Cred::userpass_plaintext(username, token)
    });

    let mut fetch_options = git2::FetchOptions::new();
    fetch_options.remote_callbacks(callbacks);

    git2::build::RepoBuilder::new()
        .fetch_options(fetch_options)
        .clone(url, path)
        .with_context(|| format!("Failed to clone {}", url))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sync_source_detect_local_explicit() {
        match SyncSource::detect("./my-project") {
            SyncSource::Local { path } => assert_eq!(path, PathBuf::from("./my-project")),
            other => panic!("Expected Local, got {:?}", other),
        }

        match SyncSource::detect("/absolute/path") {
            SyncSource::Local { path } => assert_eq!(path, PathBuf::from("/absolute/path")),
            other => panic!("Expected Local, got {:?}", other),
        }
    }

    #[test]
    fn test_sync_source_detect_github_org() {
        match SyncSource::detect("pinsky-three") {
            SyncSource::GitHubOrg { org } => assert_eq!(org, "pinsky-three"),
            SyncSource::Local { .. } => {} // OK if path exists locally
            other => panic!("Expected GitHubOrg, got {:?}", other),
        }
    }

    #[test]
    fn test_sync_source_detect_github_repo() {
        match SyncSource::detect("pinsky-three/vibe-graph") {
            SyncSource::GitHubRepo { owner, repo } => {
                assert_eq!(owner, "pinsky-three");
                assert_eq!(repo, "vibe-graph");
            }
            other => panic!("Expected GitHubRepo, got {:?}", other),
        }
    }

    #[test]
    fn test_sync_source_detect_github_url() {
        match SyncSource::detect("https://github.com/pinsky-three/vibe-graph") {
            SyncSource::GitHubRepo { owner, repo } => {
                assert_eq!(owner, "pinsky-three");
                assert_eq!(repo, "vibe-graph");
            }
            other => panic!("Expected GitHubRepo, got {:?}", other),
        }

        match SyncSource::detect("github.com/pinsky-three/vibe-graph.git") {
            SyncSource::GitHubRepo { owner, repo } => {
                assert_eq!(owner, "pinsky-three");
                assert_eq!(repo, "vibe-graph");
            }
            other => panic!("Expected GitHubRepo, got {:?}", other),
        }
    }
}
