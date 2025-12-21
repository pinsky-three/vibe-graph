//! Git fossilization helpers and real-time change detection.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use git2::{Repository, Status, StatusOptions};
use vibe_graph_core::{GitChangeKind, GitChangeSnapshot, GitFileChange, Snapshot};

/// Abstraction describing how snapshots are persisted and retrieved.
pub trait GitFossilStore {
    /// Persist the provided snapshot into the fossil store.
    fn commit_snapshot(&self, snapshot: &Snapshot) -> Result<()>;

    /// Retrieve the latest snapshot if one exists.
    fn get_latest_snapshot(&self) -> Result<Option<Snapshot>>;
}

/// Default filesystem-backed Git store.
pub struct GitBackend {
    /// Filesystem path to the repository managed by this backend.
    pub repo_path: PathBuf,
}

impl GitBackend {
    /// Construct a backend targeting the provided repository path.
    pub fn new(repo_path: PathBuf) -> Self {
        Self { repo_path }
    }
}

impl GitFossilStore for GitBackend {
    fn commit_snapshot(&self, _snapshot: &Snapshot) -> Result<()> {
        // Placeholder for future git2/plumbing integration.
        Ok(())
    }

    fn get_latest_snapshot(&self) -> Result<Option<Snapshot>> {
        Ok(None)
    }
}

// =============================================================================
// Git Change Watcher
// =============================================================================

/// Configuration for the git watcher.
#[derive(Debug, Clone)]
pub struct GitWatcherConfig {
    /// Minimum interval between polls (to avoid hammering the filesystem).
    pub min_poll_interval: Duration,
    /// Whether to include untracked files.
    pub include_untracked: bool,
    /// Whether to include ignored files.
    pub include_ignored: bool,
    /// Whether to recurse into submodules.
    pub recurse_submodules: bool,
}

impl Default for GitWatcherConfig {
    fn default() -> Self {
        Self {
            min_poll_interval: Duration::from_millis(500),
            include_untracked: true,
            include_ignored: false,
            recurse_submodules: false,
        }
    }
}

/// Watches a git repository for changes.
///
/// Uses polling-based approach with git2 for efficient status checks.
pub struct GitWatcher {
    /// Path to the repository root.
    repo_path: PathBuf,
    /// Configuration.
    config: GitWatcherConfig,
    /// Last poll time.
    last_poll: Option<Instant>,
    /// Cached snapshot.
    cached_snapshot: GitChangeSnapshot,
}

impl GitWatcher {
    /// Create a new watcher for the given repository path.
    pub fn new(repo_path: impl Into<PathBuf>) -> Self {
        Self {
            repo_path: repo_path.into(),
            config: GitWatcherConfig::default(),
            last_poll: None,
            cached_snapshot: GitChangeSnapshot::default(),
        }
    }

    /// Create with custom configuration.
    pub fn with_config(repo_path: impl Into<PathBuf>, config: GitWatcherConfig) -> Self {
        Self {
            repo_path: repo_path.into(),
            config,
            last_poll: None,
            cached_snapshot: GitChangeSnapshot::default(),
        }
    }

    /// Get the repository path.
    pub fn repo_path(&self) -> &Path {
        &self.repo_path
    }

    /// Check if it's time to poll again.
    pub fn should_poll(&self) -> bool {
        match self.last_poll {
            Some(last) => last.elapsed() >= self.config.min_poll_interval,
            None => true,
        }
    }

    /// Get the cached snapshot (may be stale).
    pub fn cached_snapshot(&self) -> &GitChangeSnapshot {
        &self.cached_snapshot
    }

    /// Poll for changes, returning the current snapshot.
    ///
    /// This is rate-limited by `min_poll_interval`. If called too frequently,
    /// returns the cached snapshot.
    pub fn poll(&mut self) -> Result<&GitChangeSnapshot> {
        if !self.should_poll() {
            return Ok(&self.cached_snapshot);
        }

        self.cached_snapshot = self.fetch_changes()?;
        self.last_poll = Some(Instant::now());
        Ok(&self.cached_snapshot)
    }

    /// Force fetch changes regardless of rate limiting.
    pub fn force_poll(&mut self) -> Result<&GitChangeSnapshot> {
        self.cached_snapshot = self.fetch_changes()?;
        self.last_poll = Some(Instant::now());
        Ok(&self.cached_snapshot)
    }

    /// Fetch current git status and convert to GitChangeSnapshot.
    fn fetch_changes(&self) -> Result<GitChangeSnapshot> {
        let repo = Repository::open(&self.repo_path)
            .with_context(|| format!("Failed to open repository at {:?}", self.repo_path))?;

        let mut opts = StatusOptions::new();
        opts.include_untracked(self.config.include_untracked)
            .include_ignored(self.config.include_ignored)
            .recurse_untracked_dirs(true)
            .exclude_submodules(true);

        let statuses = repo
            .statuses(Some(&mut opts))
            .context("Failed to get repository status")?;

        let mut changes = Vec::new();

        for entry in statuses.iter() {
            let path = match entry.path() {
                Some(p) => PathBuf::from(p),
                None => continue,
            };

            let status = entry.status();

            // Map git2 status flags to our GitChangeKind
            // Check staged changes first (index)
            if status.contains(Status::INDEX_NEW) {
                changes.push(GitFileChange {
                    path: path.clone(),
                    kind: GitChangeKind::Added,
                    staged: true,
                });
            } else if status.contains(Status::INDEX_MODIFIED) {
                changes.push(GitFileChange {
                    path: path.clone(),
                    kind: GitChangeKind::Modified,
                    staged: true,
                });
            } else if status.contains(Status::INDEX_DELETED) {
                changes.push(GitFileChange {
                    path: path.clone(),
                    kind: GitChangeKind::Deleted,
                    staged: true,
                });
            } else if status.contains(Status::INDEX_RENAMED) {
                changes.push(GitFileChange {
                    path: path.clone(),
                    kind: GitChangeKind::RenamedTo,
                    staged: true,
                });
            }

            // Check working directory changes (not yet staged)
            if status.contains(Status::WT_NEW) {
                changes.push(GitFileChange {
                    path: path.clone(),
                    kind: GitChangeKind::Untracked,
                    staged: false,
                });
            } else if status.contains(Status::WT_MODIFIED) {
                changes.push(GitFileChange {
                    path: path.clone(),
                    kind: GitChangeKind::Modified,
                    staged: false,
                });
            } else if status.contains(Status::WT_DELETED) {
                changes.push(GitFileChange {
                    path: path.clone(),
                    kind: GitChangeKind::Deleted,
                    staged: false,
                });
            } else if status.contains(Status::WT_RENAMED) {
                changes.push(GitFileChange {
                    path: path.clone(),
                    kind: GitChangeKind::RenamedTo,
                    staged: false,
                });
            }
        }

        Ok(GitChangeSnapshot {
            changes,
            captured_at: Some(Instant::now()),
        })
    }
}

/// Quick helper to get current changes for a path.
pub fn get_git_changes(repo_path: &Path) -> Result<GitChangeSnapshot> {
    let mut watcher = GitWatcher::new(repo_path);
    watcher.force_poll().cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn init_test_repo() -> Result<(TempDir, Repository)> {
        let dir = TempDir::new()?;
        let repo = Repository::init(dir.path())?;
        Ok((dir, repo))
    }

    #[test]
    fn test_watcher_empty_repo() -> Result<()> {
        let (dir, _repo) = init_test_repo()?;
        let mut watcher = GitWatcher::new(dir.path());
        let snapshot = watcher.force_poll()?;
        assert!(snapshot.changes.is_empty());
        Ok(())
    }

    #[test]
    fn test_watcher_detects_new_file() -> Result<()> {
        let (dir, _repo) = init_test_repo()?;
        fs::write(dir.path().join("new_file.txt"), "hello")?;

        let mut watcher = GitWatcher::new(dir.path());
        let snapshot = watcher.force_poll()?;

        assert_eq!(snapshot.changes.len(), 1);
        assert_eq!(snapshot.changes[0].kind, GitChangeKind::Untracked);
        assert!(!snapshot.changes[0].staged);
        Ok(())
    }

    #[test]
    fn test_watcher_rate_limiting() -> Result<()> {
        let (dir, _repo) = init_test_repo()?;
        let config = GitWatcherConfig {
            min_poll_interval: Duration::from_secs(60), // Long interval
            ..Default::default()
        };
        let mut watcher = GitWatcher::with_config(dir.path(), config);

        // First poll should work
        assert!(watcher.should_poll());
        watcher.poll()?;

        // Second poll should be rate-limited
        assert!(!watcher.should_poll());
        Ok(())
    }
}
