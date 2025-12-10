//! Organization command implementation.
//!
//! Handles operations across GitHub organizations or namespaces.

use std::path::Path;

use anyhow::{Context, Result};
use git2::{Cred, RemoteCallbacks};
use octocrab::Octocrab;
use tracing::{info, warn};
use walkdir::WalkDir;

use crate::config::Config;
use crate::project::{Project, ProjectSource, Repository, Source};

/// List repositories in an organization.
pub async fn list(config: &Config, org: &str) -> Result<()> {
    config.validate_github()?;

    let octocrab = Octocrab::builder()
        .personal_token(config.github_token.clone().unwrap())
        .build()?;

    println!("Fetching repositories for organization: {}", org);

    let repos = octocrab
        .orgs(org)
        .list_repos()
        .per_page(100)
        .page(0u32)
        .send()
        .await
        .with_context(|| format!("Failed to fetch repos for org: {}", org))?;

    println!("\nRepositories ({} found):", repos.items.len());
    println!("{:-<60}", "");

    for (i, repo) in repos.items.iter().enumerate() {
        let visibility = if repo.private.unwrap_or(false) {
            "private"
        } else {
            "public"
        };
        let description = repo
            .description
            .as_deref()
            .unwrap_or("")
            .chars()
            .take(50)
            .collect::<String>();

        println!(
            "{:3}. {:30} [{:7}] {}",
            i + 1,
            repo.name,
            visibility,
            description
        );
    }

    Ok(())
}

/// Clone all repositories from an organization.
pub async fn clone(config: &Config, org: &str, ignore_list: &[String]) -> Result<Project> {
    config.validate_github()?;

    let octocrab = Octocrab::builder()
        .personal_token(config.github_token.clone().unwrap())
        .build()?;

    info!(org = %org, "Fetching organization repositories");

    let repos = octocrab
        .orgs(org)
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
    let cache_dir = config.org_cache_dir(org);

    // Ensure cache directory exists
    std::fs::create_dir_all(&cache_dir)
        .with_context(|| format!("Failed to create cache dir: {}", cache_dir.display()))?;

    println!("Cloning repositories to: {}", cache_dir.display());

    for (i, repo) in repos.items.iter().enumerate() {
        let repo_name = &repo.name;

        // Skip if in ignore list
        if ignore_list.iter().any(|s| s == repo_name) {
            println!(
                "[{}/{}] Skipping {} (in ignore list)",
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
            println!("[{}/{}] Cloning {}...", i + 1, repos.items.len(), repo_name);

            // Clean up incomplete clone if exists
            if repo_path.exists() {
                std::fs::remove_dir_all(&repo_path)?;
            }

            clone_repository(&clone_url, &repo_path, &username, &token)?;
        } else {
            println!(
                "[{}/{}] Using cached {}",
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

    println!(
        "\nCloned {} repositories ({} files total)",
        project.repositories.len(),
        project.total_sources()
    );

    Ok(project)
}

/// Sync (pull latest) all repositories in an organization.
#[allow(dead_code)]
pub async fn sync(config: &Config, org: &str) -> Result<()> {
    let cache_dir = config.org_cache_dir(org);

    if !cache_dir.exists() {
        anyhow::bail!("Organization not cloned. Run `vg org clone {}` first.", org);
    }

    println!("Syncing repositories in: {}", cache_dir.display());

    for entry in std::fs::read_dir(&cache_dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() && path.join(".git").exists() {
            let repo_name = path.file_name().unwrap().to_string_lossy();
            print!("Syncing {}... ", repo_name);

            match git2::Repository::open(&path) {
                Ok(repo) => {
                    // Get current branch
                    match repo.head() {
                        Ok(head) => {
                            if let Some(branch) = head.shorthand() {
                                println!("on branch {}", branch);
                            } else {
                                println!("(detached HEAD)");
                            }
                        }
                        Err(e) => {
                            warn!("Failed to get HEAD: {}", e);
                            println!("(unknown state)");
                        }
                    }
                }
                Err(e) => {
                    warn!("Failed to open repo: {}", e);
                    println!("FAILED");
                }
            }
        }
    }

    Ok(())
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
