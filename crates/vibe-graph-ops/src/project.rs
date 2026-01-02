//! Domain types for projects, repositories, and sources.

use std::path::PathBuf;

use file_format::FileFormat;
use humansize::{format_size, DECIMAL};
use serde::{Deserialize, Serialize};

use crate::error::OpsResult;

/// A project represents a collection of repositories.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    /// Name of the project.
    pub name: String,

    /// Type of project source.
    pub source: ProjectSource,

    /// All repositories in this project.
    pub repositories: Vec<Repository>,
}

/// Where the project originates from.
///
/// Note: Uses default externally tagged serde format for backward compatibility
/// with existing `.self/project.json` files (e.g., `{"LocalPaths": {...}}`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ProjectSource {
    /// A GitHub organization.
    GitHubOrg { organization: String },
    /// A single GitHub repository.
    GitHubRepo { owner: String, repo: String },
    /// A local directory (single repo).
    LocalPath { path: PathBuf },
    /// Multiple local directories.
    LocalPaths { paths: Vec<PathBuf> },
}

impl Project {
    /// Create a new project for a GitHub organization.
    pub fn github_org(organization: impl Into<String>) -> Self {
        let org = organization.into();
        Self {
            name: org.clone(),
            source: ProjectSource::GitHubOrg { organization: org },
            repositories: vec![],
        }
    }

    /// Create a new project for a single GitHub repository.
    pub fn github_repo(owner: impl Into<String>, repo: impl Into<String>) -> Self {
        let owner = owner.into();
        let repo = repo.into();
        Self {
            name: repo.clone(),
            source: ProjectSource::GitHubRepo { owner, repo },
            repositories: vec![],
        }
    }

    /// Create a new project from a local path.
    pub fn local(path: PathBuf) -> Self {
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "local".to_string());
        Self {
            name,
            source: ProjectSource::LocalPath { path },
            repositories: vec![],
        }
    }

    /// Create a new project from multiple local paths.
    pub fn local_paths(name: impl Into<String>, paths: Vec<PathBuf>) -> Self {
        Self {
            name: name.into(),
            source: ProjectSource::LocalPaths { paths },
            repositories: vec![],
        }
    }

    /// Expand content for sources matching the filter predicate.
    pub fn expand_content<F>(&mut self, filter_fn: F) -> OpsResult<()>
    where
        F: Fn(&Source) -> bool,
    {
        for repo in &mut self.repositories {
            for source in &mut repo.sources {
                if filter_fn(source) {
                    source.content = std::fs::read_to_string(&source.path).ok();
                }
            }
        }
        Ok(())
    }

    /// Get total count of all sources across repositories.
    pub fn total_sources(&self) -> usize {
        self.repositories.iter().map(|r| r.sources.len()).sum()
    }

    /// Get total size of all sources.
    pub fn total_size(&self) -> u64 {
        self.repositories
            .iter()
            .flat_map(|r| &r.sources)
            .filter_map(|s| s.size)
            .sum()
    }

    /// Get human-readable total size.
    pub fn human_total_size(&self) -> String {
        format_size(self.total_size(), DECIMAL)
    }
}

/// A repository within a project.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Repository {
    /// Repository name.
    pub name: String,

    /// Clone URL (for remote repos) or local path.
    pub url: String,

    /// Local path where the repository is checked out.
    pub local_path: PathBuf,

    /// All source files in this repository.
    pub sources: Vec<Source>,
}

impl Repository {
    /// Create a new repository.
    pub fn new(name: impl Into<String>, url: impl Into<String>, local_path: PathBuf) -> Self {
        Self {
            name: name.into(),
            url: url.into(),
            local_path,
            sources: vec![],
        }
    }

    /// Get total size of all sources in this repository.
    pub fn total_size(&self) -> u64 {
        self.sources.iter().filter_map(|s| s.size).sum()
    }

    /// Get human-readable total size.
    pub fn human_total_size(&self) -> String {
        format_size(self.total_size(), DECIMAL)
    }
}

/// A source file within a repository.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Source {
    /// Absolute path to the file.
    pub path: PathBuf,

    /// Path relative to the repository root.
    pub relative_path: String,

    /// Detected file format.
    #[serde(with = "file_format_serde")]
    pub format: FileFormat,

    /// File size in bytes.
    pub size: Option<u64>,

    /// File content (populated on demand).
    pub content: Option<String>,
}

impl Source {
    /// Create a new source from a file path.
    pub fn from_path(path: PathBuf, repo_root: &PathBuf) -> OpsResult<Self> {
        let relative_path = path
            .strip_prefix(repo_root)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| path.to_string_lossy().to_string());

        let format = FileFormat::from_file(&path).unwrap_or(FileFormat::ArbitraryBinaryData);
        let size = std::fs::metadata(&path).ok().map(|m| m.len());

        Ok(Self {
            path,
            relative_path,
            format,
            size,
            content: None,
        })
    }

    /// Get human-readable size.
    pub fn human_size(&self) -> String {
        self.size
            .map(|s| format_size(s, DECIMAL))
            .unwrap_or_else(|| "unknown".to_string())
    }

    /// Check if this source appears to be a text file.
    pub fn is_text(&self) -> bool {
        matches!(
            self.format,
            FileFormat::ArbitraryBinaryData | FileFormat::PlainText
        )
    }

    /// Get the file extension if available.
    pub fn extension(&self) -> Option<&str> {
        self.path.extension().and_then(|e| e.to_str())
    }
}

/// Custom serde for FileFormat.
mod file_format_serde {
    use file_format::FileFormat;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(format: &FileFormat, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        format.to_string().serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<FileFormat, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(if s == "Plain Text" {
            FileFormat::PlainText
        } else {
            FileFormat::ArbitraryBinaryData
        })
    }
}
