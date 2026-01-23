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

// =============================================================================
// Git Command Execution
// =============================================================================

use git2::{IndexAddOption, Signature, Time};
use serde::{Deserialize, Serialize};

/// Result of a git add operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitAddResult {
    /// Files that were staged.
    pub staged_files: Vec<PathBuf>,
    /// Number of files staged.
    pub count: usize,
}

/// Result of a git commit operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitCommitResult {
    /// The commit hash (SHA).
    pub commit_id: String,
    /// The commit message.
    pub message: String,
    /// Number of files in the commit.
    pub file_count: usize,
}

/// Result of a git reset operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitResetResult {
    /// Files that were unstaged.
    pub unstaged_files: Vec<PathBuf>,
    /// Number of files unstaged.
    pub count: usize,
}

/// Branch information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitBranch {
    /// Branch name.
    pub name: String,
    /// Whether this is the current branch.
    pub is_current: bool,
    /// Whether this is a remote branch.
    pub is_remote: bool,
    /// Latest commit SHA on this branch.
    pub commit_id: Option<String>,
}

/// Result of listing branches.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitBranchListResult {
    /// All branches.
    pub branches: Vec<GitBranch>,
    /// Current branch name (if any).
    pub current: Option<String>,
}

/// Commit log entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitLogEntry {
    /// Commit SHA.
    pub commit_id: String,
    /// Short SHA (7 chars).
    pub short_id: String,
    /// Commit message.
    pub message: String,
    /// Author name.
    pub author: String,
    /// Author email.
    pub author_email: String,
    /// Unix timestamp.
    pub timestamp: i64,
}

/// Result of git log operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitLogResult {
    /// Commit entries.
    pub commits: Vec<GitLogEntry>,
}

/// Result of git diff operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitDiffResult {
    /// The diff output as text.
    pub diff: String,
    /// Number of files changed.
    pub files_changed: usize,
    /// Lines added.
    pub insertions: usize,
    /// Lines removed.
    pub deletions: usize,
}

/// Stage files in the git index.
///
/// If `paths` is empty, stages all modified/untracked files (like `git add -A`).
pub fn git_add(repo_path: &Path, paths: &[PathBuf]) -> Result<GitAddResult> {
    let repo = Repository::open(repo_path)
        .with_context(|| format!("Failed to open repository at {:?}", repo_path))?;

    let mut index = repo.index().context("Failed to get repository index")?;

    let staged_files = if paths.is_empty() {
        // Stage all changes (git add -A)
        index
            .add_all(["*"].iter(), IndexAddOption::DEFAULT, None)
            .context("Failed to add all files to index")?;

        // Get list of staged files
        let statuses = repo.statuses(None)?;
        statuses
            .iter()
            .filter_map(|e| e.path().map(PathBuf::from))
            .collect()
    } else {
        // Stage specific files
        for path in paths {
            // Convert to repo-relative path
            let rel_path = if path.is_absolute() {
                path.strip_prefix(repo_path).unwrap_or(path)
            } else {
                path.as_path()
            };
            index
                .add_path(rel_path)
                .with_context(|| format!("Failed to add {:?} to index", rel_path))?;
        }
        paths.to_vec()
    };

    index.write().context("Failed to write index")?;

    let count = staged_files.len();
    Ok(GitAddResult {
        staged_files,
        count,
    })
}

/// Create a commit with the staged changes.
pub fn git_commit(repo_path: &Path, message: &str) -> Result<GitCommitResult> {
    let repo = Repository::open(repo_path)
        .with_context(|| format!("Failed to open repository at {:?}", repo_path))?;

    let mut index = repo.index().context("Failed to get repository index")?;

    // Check if there are staged changes
    let statuses = repo.statuses(None)?;
    let staged_count = statuses
        .iter()
        .filter(|e| {
            let s = e.status();
            s.contains(Status::INDEX_NEW)
                || s.contains(Status::INDEX_MODIFIED)
                || s.contains(Status::INDEX_DELETED)
                || s.contains(Status::INDEX_RENAMED)
        })
        .count();

    if staged_count == 0 {
        anyhow::bail!("Nothing to commit - no staged changes");
    }

    // Write tree from index
    let tree_id = index.write_tree().context("Failed to write tree")?;
    let tree = repo.find_tree(tree_id).context("Failed to find tree")?;

    // Get signature (author/committer)
    let signature = repo
        .signature()
        .or_else(|_| {
            // Fallback signature if not configured
            Signature::new(
                "Vibe Graph",
                "vibe-graph@local",
                &Time::new(chrono_timestamp(), 0),
            )
        })
        .context("Failed to get signature")?;

    // Get parent commit (if any)
    let parent_commit = match repo.head() {
        Ok(head) => Some(
            head.peel_to_commit()
                .context("Failed to get parent commit")?,
        ),
        Err(_) => None, // Initial commit
    };

    let parents: Vec<&git2::Commit> = parent_commit.iter().collect();

    // Create commit
    let commit_id = repo
        .commit(
            Some("HEAD"),
            &signature,
            &signature,
            message,
            &tree,
            &parents,
        )
        .context("Failed to create commit")?;

    Ok(GitCommitResult {
        commit_id: commit_id.to_string(),
        message: message.to_string(),
        file_count: staged_count,
    })
}

/// Unstage files from the index.
///
/// If `paths` is empty, unstages all files (like `git reset HEAD`).
pub fn git_reset(repo_path: &Path, paths: &[PathBuf]) -> Result<GitResetResult> {
    let repo = Repository::open(repo_path)
        .with_context(|| format!("Failed to open repository at {:?}", repo_path))?;

    let head = repo.head().ok();
    let head_commit = head.as_ref().and_then(|h| h.peel_to_commit().ok());

    let unstaged_files = if paths.is_empty() {
        // Get list of staged files BEFORE resetting
        let statuses = repo.statuses(None)?;
        let staged_files: Vec<PathBuf> = statuses
            .iter()
            .filter(|e| {
                let s = e.status();
                s.contains(Status::INDEX_NEW)
                    || s.contains(Status::INDEX_MODIFIED)
                    || s.contains(Status::INDEX_DELETED)
            })
            .filter_map(|e| e.path().map(PathBuf::from))
            .collect();

        // Reset all staged files by passing them explicitly
        if !staged_files.is_empty() {
            if let Some(commit) = &head_commit {
                let path_refs: Vec<&Path> = staged_files.iter().map(|p| p.as_path()).collect();
                repo.reset_default(Some(commit.as_object()), path_refs.iter().copied())
                    .context("Failed to reset index")?;
            } else {
                // No HEAD commit (initial repo) - reset index to empty
                let mut index = repo.index()?;
                for path in &staged_files {
                    let _ = index.remove_path(path);
                }
                index.write()?;
            }
        }

        staged_files
    } else {
        // Reset specific files
        if let Some(commit) = &head_commit {
            let path_refs: Vec<&Path> = paths.iter().map(|p| p.as_path()).collect();
            repo.reset_default(Some(commit.as_object()), path_refs.iter().copied())
                .context("Failed to reset files")?;
        }
        paths.to_vec()
    };

    let count = unstaged_files.len();
    Ok(GitResetResult {
        unstaged_files,
        count,
    })
}

/// List all branches in the repository.
pub fn git_list_branches(repo_path: &Path) -> Result<GitBranchListResult> {
    let repo = Repository::open(repo_path)
        .with_context(|| format!("Failed to open repository at {:?}", repo_path))?;

    let mut branches = Vec::new();
    let mut current_branch = None;

    // Get current branch name
    if let Ok(head) = repo.head() {
        if head.is_branch() {
            current_branch = head.shorthand().map(String::from);
        }
    }

    // Iterate all branches
    for branch_result in repo.branches(None)? {
        let (branch, branch_type) = branch_result?;
        let name = branch.name()?.unwrap_or("").to_string();
        let is_remote = matches!(branch_type, git2::BranchType::Remote);
        let is_current = Some(&name) == current_branch.as_ref();

        let commit_id = branch
            .get()
            .peel_to_commit()
            .ok()
            .map(|c| c.id().to_string());

        branches.push(GitBranch {
            name,
            is_current,
            is_remote,
            commit_id,
        });
    }

    Ok(GitBranchListResult {
        branches,
        current: current_branch,
    })
}

/// Checkout a branch.
pub fn git_checkout_branch(repo_path: &Path, branch_name: &str) -> Result<()> {
    let repo = Repository::open(repo_path)
        .with_context(|| format!("Failed to open repository at {:?}", repo_path))?;

    // Find the branch
    let branch = repo
        .find_branch(branch_name, git2::BranchType::Local)
        .with_context(|| format!("Branch '{}' not found", branch_name))?;

    let reference = branch.get();
    let commit = reference
        .peel_to_commit()
        .context("Failed to get commit for branch")?;

    // Checkout the tree
    let tree = commit.tree().context("Failed to get tree")?;
    repo.checkout_tree(tree.as_object(), None)
        .context("Failed to checkout tree")?;

    // Set HEAD to the branch
    repo.set_head(reference.name().unwrap_or(""))
        .context("Failed to set HEAD")?;

    Ok(())
}

/// Get commit log.
pub fn git_log(repo_path: &Path, limit: usize) -> Result<GitLogResult> {
    let repo = Repository::open(repo_path)
        .with_context(|| format!("Failed to open repository at {:?}", repo_path))?;

    let mut revwalk = repo.revwalk().context("Failed to create revwalk")?;
    revwalk.push_head().context("Failed to push HEAD")?;

    let mut commits = Vec::new();

    for (i, oid_result) in revwalk.enumerate() {
        if i >= limit {
            break;
        }

        let oid = oid_result.context("Failed to get commit OID")?;
        let commit = repo.find_commit(oid).context("Failed to find commit")?;

        let author = commit.author();
        commits.push(GitLogEntry {
            commit_id: oid.to_string(),
            short_id: oid.to_string()[..7].to_string(),
            message: commit.message().unwrap_or("").to_string(),
            author: author.name().unwrap_or("Unknown").to_string(),
            author_email: author.email().unwrap_or("").to_string(),
            timestamp: author.when().seconds(),
        });
    }

    Ok(GitLogResult { commits })
}

/// Get diff of staged changes or working directory.
pub fn git_diff(repo_path: &Path, staged: bool) -> Result<GitDiffResult> {
    let repo = Repository::open(repo_path)
        .with_context(|| format!("Failed to open repository at {:?}", repo_path))?;

    let mut diff_opts = git2::DiffOptions::new();
    diff_opts.include_untracked(true);

    let diff = if staged {
        // Diff between HEAD and index (staged changes)
        let head_tree = repo.head().ok().and_then(|h| h.peel_to_tree().ok());
        repo.diff_tree_to_index(head_tree.as_ref(), None, Some(&mut diff_opts))
            .context("Failed to get staged diff")?
    } else {
        // Diff between index and working directory (unstaged changes)
        repo.diff_index_to_workdir(None, Some(&mut diff_opts))
            .context("Failed to get working directory diff")?
    };

    let stats = diff.stats().context("Failed to get diff stats")?;

    let mut diff_text = String::new();
    diff.print(git2::DiffFormat::Patch, |_delta, _hunk, line| {
        let prefix = match line.origin() {
            '+' => "+",
            '-' => "-",
            ' ' => " ",
            _ => "",
        };
        diff_text.push_str(prefix);
        diff_text.push_str(std::str::from_utf8(line.content()).unwrap_or(""));
        true
    })
    .context("Failed to print diff")?;

    Ok(GitDiffResult {
        diff: diff_text,
        files_changed: stats.files_changed(),
        insertions: stats.insertions(),
        deletions: stats.deletions(),
    })
}

/// Helper to get current unix timestamp.
fn chrono_timestamp() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
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

    // =========================================================================
    // Git Command Tests
    // =========================================================================

    #[test]
    fn test_git_add_and_commit() -> Result<()> {
        let (dir, repo) = init_test_repo()?;

        // Configure user for commit
        let mut config = repo.config()?;
        config.set_str("user.name", "Test User")?;
        config.set_str("user.email", "test@example.com")?;

        // Create a file
        fs::write(dir.path().join("test.txt"), "hello world")?;

        // Stage the file
        let add_result = git_add(dir.path(), &[])?;
        assert_eq!(add_result.count, 1);

        // Commit
        let commit_result = git_commit(dir.path(), "Initial commit")?;
        assert_eq!(commit_result.message, "Initial commit");
        assert!(!commit_result.commit_id.is_empty());

        Ok(())
    }

    #[test]
    fn test_git_commit_fails_without_staged() -> Result<()> {
        let (dir, repo) = init_test_repo()?;

        // Configure user for commit
        let mut config = repo.config()?;
        config.set_str("user.name", "Test User")?;
        config.set_str("user.email", "test@example.com")?;

        // Try to commit without staged changes
        let result = git_commit(dir.path(), "Empty commit");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Nothing to commit"));

        Ok(())
    }

    #[test]
    fn test_git_branches() -> Result<()> {
        let (dir, repo) = init_test_repo()?;

        // Configure user
        let mut config = repo.config()?;
        config.set_str("user.name", "Test User")?;
        config.set_str("user.email", "test@example.com")?;

        // Create initial commit (needed for branches to exist)
        fs::write(dir.path().join("test.txt"), "hello")?;
        git_add(dir.path(), &[])?;
        git_commit(dir.path(), "Initial")?;

        // List branches
        let branches = git_list_branches(dir.path())?;
        assert!(!branches.branches.is_empty());

        // Default branch should be current
        let current = branches.branches.iter().find(|b| b.is_current);
        assert!(current.is_some());

        Ok(())
    }

    #[test]
    fn test_git_log() -> Result<()> {
        let (dir, repo) = init_test_repo()?;

        // Configure user
        let mut config = repo.config()?;
        config.set_str("user.name", "Test User")?;
        config.set_str("user.email", "test@example.com")?;

        // Create commits
        fs::write(dir.path().join("test.txt"), "hello")?;
        git_add(dir.path(), &[])?;
        git_commit(dir.path(), "First commit")?;

        fs::write(dir.path().join("test.txt"), "hello world")?;
        git_add(dir.path(), &[])?;
        git_commit(dir.path(), "Second commit")?;

        // Get log
        let log = git_log(dir.path(), 10)?;
        assert_eq!(log.commits.len(), 2);
        assert_eq!(log.commits[0].message.trim(), "Second commit");
        assert_eq!(log.commits[1].message.trim(), "First commit");

        Ok(())
    }

    #[test]
    fn test_git_diff() -> Result<()> {
        let (dir, repo) = init_test_repo()?;

        // Configure user
        let mut config = repo.config()?;
        config.set_str("user.name", "Test User")?;
        config.set_str("user.email", "test@example.com")?;

        // Create initial commit
        fs::write(dir.path().join("test.txt"), "hello")?;
        git_add(dir.path(), &[])?;
        git_commit(dir.path(), "Initial")?;

        // Modify file
        fs::write(dir.path().join("test.txt"), "hello world")?;

        // Get working directory diff
        let diff = git_diff(dir.path(), false)?;
        assert_eq!(diff.files_changed, 1);
        assert!(diff.insertions > 0 || diff.deletions > 0);

        // Stage and get staged diff
        git_add(dir.path(), &[])?;
        let staged_diff = git_diff(dir.path(), true)?;
        assert_eq!(staged_diff.files_changed, 1);

        Ok(())
    }
}
