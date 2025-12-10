//! Scan command implementation.
//!
//! Scans a local repository and emits a summary of the source graph.

use std::path::{Path, PathBuf};

use anyhow::Result;
use tracing::info;
use vibe_graph_ssot::{LocalFsScanner, SourceScanner};
use walkdir::WalkDir;

use crate::project::{Project, Repository, Source};

/// Execute the scan command on a local path.
pub fn execute(path: PathBuf, verbose: bool) -> Result<()> {
    // Canonicalize path to handle relative paths
    let path = path.canonicalize().unwrap_or(path);
    info!(path = %path.display(), "Scanning repository");

    // Use vibe-graph-ssot scanner for graph construction
    let scanner = LocalFsScanner;
    let graph = scanner.scan_repo(&path)?;

    println!(
        "Graph summary: {} nodes, {} edges",
        graph.node_count(),
        graph.edge_count()
    );

    // Also do a detailed file scan
    let project = scan_local_path(&path)?;

    println!("\nRepository: {}", project.name);
    println!("Total files: {}", project.total_sources());
    println!(
        "Total size: {}",
        humansize::format_size(project.total_size(), humansize::DECIMAL)
    );

    if verbose {
        for repo in &project.repositories {
            println!("\n  {} ({} files)", repo.name, repo.sources.len());
            for source in &repo.sources {
                println!(
                    "    {} ({}) - {}",
                    source.relative_path,
                    source.human_size(),
                    source.format
                );
            }
        }
    }

    Ok(())
}

/// Scan a local path and build a Project structure.
pub fn scan_local_path(path: &Path) -> Result<Project> {
    // Canonicalize to handle relative paths
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let mut project = Project::local(path.clone());

    let repo_name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "local".to_string());

    let mut repo = Repository::new(&repo_name, path.display().to_string(), path.clone());

    for entry in WalkDir::new(&path)
        .into_iter()
        .filter_entry(|e| !is_hidden(e) && !is_blacklisted(e))
        .filter_map(|e| e.ok())
    {
        if entry.path().is_file() {
            if let Ok(source) = Source::from_path(entry.path().to_path_buf(), &path) {
                repo.sources.push(source);
            }
        }
    }

    project.repositories.push(repo);
    Ok(project)
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
    ];

    entry
        .file_name()
        .to_str()
        .map(|s| BLACKLIST.contains(&s))
        .unwrap_or(false)
}
