//! Workspace detection and sync source types.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::error::{OpsError, OpsResult};

/// Detected workspace type based on directory structure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
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

/// The source type for a sync operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SyncSource {
    /// A local filesystem path.
    Local { path: PathBuf },
    /// A GitHub organization (clone all repos).
    GitHubOrg { org: String },
    /// A single GitHub repository.
    GitHubRepo { owner: String, repo: String },
}

impl SyncSource {
    /// Create a local sync source.
    pub fn local(path: impl Into<PathBuf>) -> Self {
        Self::Local { path: path.into() }
    }

    /// Create a GitHub org sync source.
    pub fn github_org(org: impl Into<String>) -> Self {
        Self::GitHubOrg { org: org.into() }
    }

    /// Create a GitHub repo sync source.
    pub fn github_repo(owner: impl Into<String>, repo: impl Into<String>) -> Self {
        Self::GitHubRepo {
            owner: owner.into(),
            repo: repo.into(),
        }
    }

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
            _ => Self::GitHubRepo {
                owner: parts[0].to_string(),
                repo: parts[1].to_string(),
            },
        }
    }

    /// Check if this is a remote source (requires GitHub API).
    pub fn is_remote(&self) -> bool {
        matches!(self, Self::GitHubOrg { .. } | Self::GitHubRepo { .. })
    }

    /// Check if this is a local source.
    pub fn is_local(&self) -> bool {
        matches!(self, Self::Local { .. })
    }

    /// Get the local path if this is a local source.
    pub fn local_path(&self) -> Option<&Path> {
        match self {
            Self::Local { path } => Some(path.as_path()),
            _ => None,
        }
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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

impl WorkspaceInfo {
    /// Detect the workspace type for a given path.
    pub fn detect(path: &Path) -> OpsResult<Self> {
        let root = path.canonicalize().map_err(|e| OpsError::PathResolution {
            path: path.to_path_buf(),
            message: e.to_string(),
        })?;

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

    /// Check if this is a single repository workspace.
    pub fn is_single_repo(&self) -> bool {
        matches!(self.kind, WorkspaceKind::SingleRepo)
    }

    /// Check if this is a multi-repo workspace.
    pub fn is_multi_repo(&self) -> bool {
        matches!(self.kind, WorkspaceKind::MultiRepo { .. })
    }
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
    }
}
