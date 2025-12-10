//! Sync command implementation.
//!
//! The primary command that auto-detects workspace structure and builds the graph.
//! Analyzes the current directory to determine if it's:
//! - A single git repository
//! - A directory containing multiple git repositories (workspace/organization)
//! - A plain directory (treated as single project)

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
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
