//! Remote repository commands.
//!
//! Handles operations with git remotes and GitHub organizations.
//!
//! - For single repos: auto-detects the git remote origin
//! - For workspaces: allows adding a GitHub org as the remote

use std::path::Path;

use anyhow::{Context, Result};
use git2::{Cred, RemoteCallbacks, Repository as GitRepo};
use octocrab::Octocrab;
use tracing::info;
use walkdir::WalkDir;

use crate::config::Config;
use crate::project::{Project, ProjectSource, Repository, Source};
use crate::store::Store;

/// Remote information for a workspace.
#[derive(Debug, Clone)]
pub struct RemoteInfo {
    /// The remote URL or organization.
    pub url: String,
    /// Whether this is a GitHub organization (vs a repo URL).
    pub is_org: bool,
    /// Parsed organization name if applicable.
    pub org_name: Option<String>,
}

impl RemoteInfo {
    /// Parse a URL or org reference into RemoteInfo.
    pub fn parse(input: &str) -> Self {
        let cleaned = input
            .trim()
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_start_matches("github.com/")
            .trim_end_matches('/')
            .trim_end_matches(".git");

        // Check if it looks like an org (no slash after domain, or single segment)
        let parts: Vec<&str> = cleaned.split('/').collect();

        if parts.len() == 1 {
            // Just an org name like "pinsky-three"
            Self {
                url: format!("https://github.com/{}", parts[0]),
                is_org: true,
                org_name: Some(parts[0].to_string()),
            }
        } else if parts.len() == 2 && !cleaned.contains(".git") {
            // Could be org/repo or just org - treat as org if no .git
            // e.g., "github.com/pinsky-three" -> org
            // e.g., "github.com/pinsky-three/vibe-graph" -> repo
            if parts[1].is_empty() || input.contains("github.com/") && !input.contains(".git") {
                Self {
                    url: format!("https://github.com/{}", parts[0]),
                    is_org: true,
                    org_name: Some(parts[0].to_string()),
                }
            } else {
                Self {
                    url: format!("https://github.com/{}/{}", parts[0], parts[1]),
                    is_org: false,
                    org_name: Some(parts[0].to_string()),
                }
            }
        } else {
            // Full URL
            Self {
                url: input.to_string(),
                is_org: false,
                org_name: extract_org_from_url(input),
            }
        }
    }
}

/// Extract organization/owner from a GitHub URL.
fn extract_org_from_url(url: &str) -> Option<String> {
    let cleaned = url
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("git@github.com:")
        .trim_start_matches("github.com/")
        .trim_end_matches(".git");

    cleaned.split('/').next().map(|s| s.to_string())
}

/// Detect git remote origin for a repository path.
pub fn detect_git_remote(repo_path: &Path) -> Option<String> {
    let git_repo = GitRepo::open(repo_path).ok()?;
    let remote = git_repo.find_remote("origin").ok()?;
    remote.url().map(|s| s.to_string())
}

/// Show remote information for the current workspace.
pub fn show(store: &Store) -> Result<()> {
    let manifest = store
        .load_manifest()?
        .ok_or_else(|| anyhow::anyhow!("No .self folder found. Run `vg sync` first."))?;

    println!("📡 Remote Configuration");
    println!("{:─<50}", "");
    println!();

    if let Some(remote) = &manifest.remote {
        let info = RemoteInfo::parse(remote);
        println!("🔗 Remote:  {}", remote);
        if info.is_org {
            println!("   Type:    GitHub Organization");
            if let Some(org) = &info.org_name {
                println!("   Org:     {}", org);
            }
        } else {
            println!("   Type:    Git Repository");
            if let Some(org) = &info.org_name {
                println!("   Owner:   {}", org);
            }
        }
    } else {
        println!("⚠️  No remote configured");
        println!();
        if manifest.kind == "single repository" {
            println!("   This repo has no git remote origin.");
            println!("   Add one with: git remote add origin <url>");
        } else {
            println!("   Add a GitHub org with: vg remote add <org-name>");
            println!("   Example: vg remote add pinsky-three");
        }
    }

    Ok(())
}

/// Add a remote to the workspace manifest.
pub fn add(store: &Store, remote_input: &str) -> Result<()> {
    let mut manifest = store
        .load_manifest()?
        .ok_or_else(|| anyhow::anyhow!("No .self folder found. Run `vg sync` first."))?;

    let info = RemoteInfo::parse(remote_input);

    // Validate that this looks like a valid remote
    if info.org_name.is_none() {
        anyhow::bail!(
            "Could not parse remote: {}. Expected format: org-name or github.com/org-name",
            remote_input
        );
    }

    manifest.remote = Some(info.url.clone());
    store.save_manifest(&manifest)?;

    println!("✅ Remote set: {}", info.url);
    if let Some(org) = &info.org_name {
        if info.is_org {
            println!("   Organization: {}", org);
            println!();
            println!("   Commands available:");
            println!("   • vg remote list    - List repositories");
            println!("   • vg remote clone   - Clone all repositories");
        }
    }

    Ok(())
}

/// Remove the remote from the workspace manifest.
pub fn remove(store: &Store) -> Result<()> {
    let mut manifest = store
        .load_manifest()?
        .ok_or_else(|| anyhow::anyhow!("No .self folder found. Run `vg sync` first."))?;

    if manifest.remote.is_none() {
        println!("ℹ️  No remote configured");
        return Ok(());
    }

    manifest.remote = None;
    store.save_manifest(&manifest)?;

    println!("✅ Remote removed");
    Ok(())
}

/// List repositories in the configured remote organization.
pub async fn list(config: &Config, store: &Store) -> Result<()> {
    config.validate_github()?;

    let manifest = store
        .load_manifest()?
        .ok_or_else(|| anyhow::anyhow!("No .self folder found. Run `vg sync` first."))?;

    let remote = manifest
        .remote
        .ok_or_else(|| anyhow::anyhow!("No remote configured. Run `vg remote add <org>` first."))?;

    let info = RemoteInfo::parse(&remote);
    let org = info
        .org_name
        .ok_or_else(|| anyhow::anyhow!("Could not determine organization from remote: {}", remote))?;

    let octocrab = Octocrab::builder()
        .personal_token(config.github_token.clone().unwrap())
        .build()?;

    println!("📋 Repositories in {}", org);
    println!("{:─<60}", "");

    let repos = octocrab
        .orgs(&org)
        .list_repos()
        .per_page(100)
        .page(0u32)
        .send()
        .await
        .with_context(|| format!("Failed to fetch repos for org: {}", org))?;

    println!();
    for (i, repo) in repos.items.iter().enumerate() {
        let visibility = if repo.private.unwrap_or(false) {
            "🔒"
        } else {
            "🌐"
        };
        let description = repo
            .description
            .as_deref()
            .unwrap_or("")
            .chars()
            .take(45)
            .collect::<String>();

        println!(
            "{:3}. {} {:30} {}",
            i + 1,
            visibility,
            repo.name,
            description
        );
    }

    println!();
    println!("Total: {} repositories", repos.items.len());

    Ok(())
}

/// Clone all repositories from the configured remote organization.
pub async fn clone(config: &Config, store: &Store, ignore_list: &[String]) -> Result<Project> {
    config.validate_github()?;

    let manifest = store
        .load_manifest()?
        .ok_or_else(|| anyhow::anyhow!("No .self folder found. Run `vg sync` first."))?;

    let remote = manifest
        .remote
        .ok_or_else(|| anyhow::anyhow!("No remote configured. Run `vg remote add <org>` first."))?;

    let info = RemoteInfo::parse(&remote);
    let org = info
        .org_name
        .ok_or_else(|| anyhow::anyhow!("Could not determine organization from remote: {}", remote))?;

    let octocrab = Octocrab::builder()
        .personal_token(config.github_token.clone().unwrap())
        .build()?;

    info!(org = %org, "Fetching organization repositories");

    let repos = octocrab
        .orgs(&org)
        .list_repos()
        .per_page(100)
        .page(0u32)
        .send()
        .await
        .with_context(|| format!("Failed to fetch repos for org: {}", org))?;

    let mut project = Project {
        name: org.to_string(),
        source: ProjectSource::GitHubOrg {
            organization: org.to_string(),
        },
        repositories: vec![],
    };

    let username = config.github_username.clone().unwrap();
    let token = config.github_token.clone().unwrap();
    let cache_dir = config.org_cache_dir(&org);

    // Ensure cache directory exists
    std::fs::create_dir_all(&cache_dir)
        .with_context(|| format!("Failed to create cache dir: {}", cache_dir.display()))?;

    println!("📥 Cloning to: {}", cache_dir.display());
    println!();

    for (i, repo) in repos.items.iter().enumerate() {
        let repo_name = &repo.name;

        // Skip if in ignore list
        if ignore_list.iter().any(|s| s == repo_name) {
            println!(
                "[{}/{}] ⏭️  Skipping {} (ignored)",
                i + 1,
                repos.items.len(),
                repo_name
            );
            continue;
        }

        let clone_url = repo
            .clone_url
            .as_ref()
            .map(|u| u.to_string())
            .unwrap_or_else(|| format!("https://github.com/{}/{}.git", org, repo_name));

        let repo_path = cache_dir.join(repo_name);

        // Clone or update repository
        if needs_clone(&repo_path) {
            println!(
                "[{}/{}] 📦 Cloning {}...",
                i + 1,
                repos.items.len(),
                repo_name
            );

            // Clean up incomplete clone if exists
            if repo_path.exists() {
                std::fs::remove_dir_all(&repo_path)?;
            }

            clone_repository(&clone_url, &repo_path, &username, &token)?;
        } else {
            println!(
                "[{}/{}] ✓  Using cached {}",
                i + 1,
                repos.items.len(),
                repo_name
            );
        }

        // Scan the repository
        let mut repository = Repository::new(repo_name, &clone_url, repo_path.clone());
        scan_repository(&mut repository)?;
        project.repositories.push(repository);
    }

    println!();
    println!(
        "✅ Cloned {} repositories ({} files total)",
        project.repositories.len(),
        project.total_sources()
    );

    Ok(project)
}

/// Check if a repository needs to be cloned.
fn needs_clone(repo_path: &Path) -> bool {
    if !repo_path.exists() {
        return true;
    }
    // Check if it's a valid git repo
    !repo_path.join(".git").exists()
}

/// Clone a repository using git2.
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

/// Scan a repository and populate its sources.
fn scan_repository(repo: &mut Repository) -> Result<()> {
    for entry in WalkDir::new(&repo.local_path)
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

/// Check if entry is hidden.
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
    ];

    entry
        .file_name()
        .to_str()
        .map(|s| BLACKLIST.contains(&s))
        .unwrap_or(false)
}
