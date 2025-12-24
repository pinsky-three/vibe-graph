//! Directory scanning utilities.

use std::path::Path;

use walkdir::WalkDir;

use crate::error::OpsResult;
use crate::project::{Repository, Source};

/// Scan a directory and populate repository sources.
pub fn scan_directory(repo: &mut Repository, path: &Path) -> OpsResult<()> {
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
        "pkg",
        ".cargo",
    ];

    entry
        .file_name()
        .to_str()
        .map(|s| BLACKLIST.contains(&s))
        .unwrap_or(false)
}

/// Customizable scan options.
#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub struct ScanOptions {
    /// Additional paths to exclude.
    pub exclude: Vec<String>,
    /// Maximum file size to include (in bytes).
    pub max_size: Option<u64>,
    /// File extensions to include (empty = all).
    pub extensions: Vec<String>,
    /// Whether to include hidden files.
    pub include_hidden: bool,
}

#[allow(dead_code)]
impl ScanOptions {
    /// Create scan options with default settings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add paths to exclude.
    pub fn exclude(mut self, paths: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.exclude.extend(paths.into_iter().map(|s| s.into()));
        self
    }

    /// Set maximum file size.
    pub fn max_size(mut self, size: u64) -> Self {
        self.max_size = Some(size);
        self
    }

    /// Filter to specific extensions.
    pub fn extensions(mut self, exts: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.extensions.extend(exts.into_iter().map(|s| s.into()));
        self
    }

    /// Include hidden files.
    pub fn include_hidden(mut self) -> Self {
        self.include_hidden = true;
        self
    }
}

/// Advanced scan with options.
#[allow(dead_code)]
pub fn scan_directory_with_options(
    repo: &mut Repository,
    path: &Path,
    options: &ScanOptions,
) -> OpsResult<()> {
    for entry in WalkDir::new(path)
        .into_iter()
        .filter_entry(|e| {
            // Check hidden
            if !options.include_hidden && is_hidden(e) {
                return false;
            }
            // Check blacklist
            if is_blacklisted(e) {
                return false;
            }
            // Check custom exclude
            if let Some(name) = e.file_name().to_str() {
                if options.exclude.iter().any(|ex| name == ex) {
                    return false;
                }
            }
            true
        })
        .filter_map(|e| e.ok())
    {
        if !entry.path().is_file() {
            continue;
        }

        // Check extensions filter
        if !options.extensions.is_empty() {
            let ext = entry
                .path()
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");
            if !options.extensions.iter().any(|e| e == ext) {
                continue;
            }
        }

        // Check size limit
        if let Some(max_size) = options.max_size {
            if let Ok(meta) = entry.metadata() {
                if meta.len() > max_size {
                    continue;
                }
            }
        }

        if let Ok(source) = Source::from_path(entry.path().to_path_buf(), &repo.local_path) {
            repo.sources.push(source);
        }
    }
    Ok(())
}

