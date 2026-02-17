//! Project configuration via `vg.toml`.
//!
//! Defines the schema for the per-project config file that tells vg
//! how to build, test, and lint a project. The config resolution chain:
//!
//! 1. Explicit `vg.toml` in the repo root
//! 2. Workspace defaults from a parent `vg.toml` `[workspace.defaults]`
//! 3. Auto-inferred from project markers (Cargo.toml, package.json, etc.)

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use crate::config::StabilityObjective;

/// Name of the project config file.
pub const CONFIG_FILENAME: &str = "vg.toml";

// =============================================================================
// Top-level config
// =============================================================================

/// The full project configuration parsed from `vg.toml`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectConfig {
    /// Project metadata.
    #[serde(default)]
    pub project: ProjectSection,

    /// Named scripts (e.g. build = "cargo build").
    #[serde(default)]
    pub scripts: HashMap<String, String>,

    /// Watch configuration for `vg run`.
    #[serde(default)]
    pub watch: WatchSection,

    /// Stability target overrides per role.
    #[serde(default)]
    pub stability: HashMap<String, f32>,

    /// Ignore patterns.
    #[serde(default)]
    pub ignore: IgnoreSection,

    /// Automaton runtime settings.
    #[serde(default)]
    pub automaton: AutomatonSection,

    /// Workspace config (only in root vg.toml for multi-repo).
    #[serde(default)]
    pub workspace: Option<WorkspaceSection>,
}

/// `[project]` section.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectSection {
    /// Project name (defaults to directory name).
    #[serde(default)]
    pub name: String,
}

/// `[watch]` section — which scripts to auto-run on change.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WatchSection {
    /// Script names to run when changes are detected.
    #[serde(default)]
    pub run: Vec<String>,
}

/// `[ignore]` section.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IgnoreSection {
    /// Directory names to skip.
    #[serde(default)]
    pub directories: Vec<String>,

    /// Glob patterns to skip.
    #[serde(default)]
    pub patterns: Vec<String>,
}

/// `[automaton]` section — runtime tuning.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutomatonSection {
    /// Maximum ticks per automaton run.
    #[serde(default = "AutomatonSection::default_max_ticks")]
    pub max_ticks: u64,

    /// Poll interval in seconds for change detection.
    #[serde(default = "AutomatonSection::default_interval")]
    pub interval: u64,
}

impl AutomatonSection {
    fn default_max_ticks() -> u64 {
        30
    }
    fn default_interval() -> u64 {
        5
    }
}

impl Default for AutomatonSection {
    fn default() -> Self {
        Self {
            max_ticks: Self::default_max_ticks(),
            interval: Self::default_interval(),
        }
    }
}

/// `[workspace]` section — multi-repo defaults.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WorkspaceSection {
    /// Default config applied to repos without their own vg.toml.
    #[serde(default)]
    pub defaults: WorkspaceDefaults,
}

/// Defaults that apply to all workspace members.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WorkspaceDefaults {
    /// Default scripts.
    #[serde(default)]
    pub scripts: HashMap<String, String>,

    /// Default watch config.
    #[serde(default)]
    pub watch: WatchSection,

    /// Default stability targets.
    #[serde(default)]
    pub stability: HashMap<String, f32>,
}

// =============================================================================
// Loading and resolution
// =============================================================================

impl ProjectConfig {
    /// Load a `vg.toml` from the given directory. Returns `None` if the file
    /// doesn't exist. Returns `Err` if the file exists but is malformed.
    pub fn load(dir: &Path) -> Result<Option<Self>, String> {
        let path = dir.join(CONFIG_FILENAME);
        if !path.exists() {
            return Ok(None);
        }

        let content = std::fs::read_to_string(&path)
            .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
        let mut config: ProjectConfig =
            toml::from_str(&content).map_err(|e| format!("Failed to parse {}: {}", path.display(), e))?;

        // Default project name to directory name
        if config.project.name.is_empty() {
            config.project.name = dir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();
        }

        info!(path = %path.display(), "Loaded project config");
        Ok(Some(config))
    }

    /// Resolve the effective config for a repo, using the resolution chain:
    ///
    /// 1. `repo_path/vg.toml` (if exists)
    /// 2. `workspace_root/vg.toml` `[workspace.defaults]` (if exists)
    /// 3. Auto-inferred from project markers
    pub fn resolve(repo_path: &Path, workspace_root: Option<&Path>) -> Self {
        // 1. Try repo-level vg.toml
        match Self::load(repo_path) {
            Ok(Some(config)) => {
                debug!(path = %repo_path.display(), "Using repo-level vg.toml");
                return config;
            }
            Ok(None) => {}
            Err(e) => {
                warn!("Error loading repo config: {}", e);
            }
        }

        // 2. Try workspace defaults
        if let Some(root) = workspace_root {
            if root != repo_path {
                match Self::load(root) {
                    Ok(Some(root_config)) => {
                        if let Some(ws) = &root_config.workspace {
                            debug!(
                                repo = %repo_path.display(),
                                root = %root.display(),
                                "Using workspace defaults"
                            );
                            return Self::from_workspace_defaults(
                                &ws.defaults,
                                repo_path,
                            );
                        }
                    }
                    Ok(None) => {}
                    Err(e) => {
                        warn!("Error loading workspace config: {}", e);
                    }
                }
            }
        }

        // 3. Auto-infer
        debug!(path = %repo_path.display(), "Inferring config from project markers");
        crate::inference::infer_config(repo_path)
    }

    /// Build a `ProjectConfig` from workspace defaults.
    fn from_workspace_defaults(defaults: &WorkspaceDefaults, repo_path: &Path) -> Self {
        let name = repo_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        ProjectConfig {
            project: ProjectSection { name },
            scripts: defaults.scripts.clone(),
            watch: defaults.watch.clone(),
            stability: defaults.stability.clone(),
            ..Default::default()
        }
    }

    /// Convert the `[stability]` section into a `StabilityObjective`.
    ///
    /// If the stability section is empty, returns the default objective.
    /// Otherwise, starts from defaults and overrides with the config values.
    pub fn stability_objective(&self) -> StabilityObjective {
        if self.stability.is_empty() {
            return StabilityObjective::default();
        }

        let mut objective = StabilityObjective::default();
        for (role, target) in &self.stability {
            objective.targets.insert(role.clone(), *target);
        }
        objective
    }

    /// Get the list of (script_name, command) pairs to run on change.
    ///
    /// Only returns scripts that are both listed in `watch.run` and
    /// defined in the `scripts` section.
    pub fn watch_scripts(&self) -> Vec<(&str, &str)> {
        self.watch
            .run
            .iter()
            .filter_map(|name| {
                self.scripts
                    .get(name.as_str())
                    .map(|cmd| (name.as_str(), cmd.as_str()))
            })
            .collect()
    }

    /// Check if this config has any scripts defined.
    pub fn has_scripts(&self) -> bool {
        !self.scripts.is_empty()
    }

    /// Check if this config has watch scripts configured.
    pub fn has_watch_scripts(&self) -> bool {
        !self.watch.run.is_empty() && self.watch.run.iter().any(|n| self.scripts.contains_key(n.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_single_repo_config() {
        let toml_str = r#"
[project]
name = "my-service"

[scripts]
build = "cargo build"
test = "cargo test"
lint = "cargo clippy -- -D warnings"
check = "cargo check"

[watch]
run = ["check", "test"]

[stability]
entry_point = 0.95
hub = 0.85
identity = 0.50

[ignore]
directories = ["node_modules", "target"]
patterns = ["*.lock"]

[automaton]
max_ticks = 30
interval = 5
"#;
        let config: ProjectConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.project.name, "my-service");
        assert_eq!(config.scripts.len(), 4);
        assert_eq!(config.scripts["test"], "cargo test");
        assert_eq!(config.watch.run, vec!["check", "test"]);
        assert_eq!(config.stability["entry_point"], 0.95);
        assert_eq!(config.ignore.directories, vec!["node_modules", "target"]);
        assert_eq!(config.automaton.max_ticks, 30);
        assert!(config.workspace.is_none());
    }

    #[test]
    fn test_parse_workspace_config() {
        let toml_str = r#"
[project]
name = "pinsky-three"

[workspace.defaults.scripts]
test = "cargo test"
lint = "cargo clippy -- -D warnings"

[workspace.defaults.watch]
run = ["test"]

[workspace.defaults.stability]
identity = 0.50
"#;
        let config: ProjectConfig = toml::from_str(toml_str).unwrap();
        assert!(config.workspace.is_some());
        let ws = config.workspace.unwrap();
        assert_eq!(ws.defaults.scripts["test"], "cargo test");
        assert_eq!(ws.defaults.watch.run, vec!["test"]);
        assert_eq!(ws.defaults.stability["identity"], 0.50);
    }

    #[test]
    fn test_parse_minimal_config() {
        let toml_str = r#"
[scripts]
test = "pytest"
"#;
        let config: ProjectConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.project.name, "");
        assert_eq!(config.scripts["test"], "pytest");
        assert!(config.watch.run.is_empty());
        assert!(config.stability.is_empty());
    }

    #[test]
    fn test_stability_objective_defaults() {
        let config = ProjectConfig::default();
        let obj = config.stability_objective();
        // Should use defaults when stability is empty
        assert_eq!(obj.target_for("entry_point"), 0.95);
        assert_eq!(obj.target_for("identity"), 0.50);
    }

    #[test]
    fn test_stability_objective_overrides() {
        let toml_str = r#"
[stability]
entry_point = 0.99
identity = 0.70
custom_role = 0.80
"#;
        let config: ProjectConfig = toml::from_str(toml_str).unwrap();
        let obj = config.stability_objective();
        assert_eq!(obj.target_for("entry_point"), 0.99);
        assert_eq!(obj.target_for("identity"), 0.70);
        assert_eq!(obj.target_for("custom_role"), 0.80);
        // Default roles not overridden should still exist
        assert_eq!(obj.target_for("hub"), 0.85);
    }

    #[test]
    fn test_watch_scripts_filters_undefined() {
        let toml_str = r#"
[scripts]
test = "cargo test"
build = "cargo build"

[watch]
run = ["test", "lint", "build"]
"#;
        let config: ProjectConfig = toml::from_str(toml_str).unwrap();
        let watch = config.watch_scripts();
        // "lint" is in watch.run but not defined in scripts, so it's skipped
        assert_eq!(watch.len(), 2);
        assert_eq!(watch[0].0, "test");
        assert_eq!(watch[1].0, "build");
    }

    #[test]
    fn test_load_from_directory() {
        let dir = tempfile::TempDir::new().unwrap();
        let toml_path = dir.path().join("vg.toml");
        std::fs::write(
            &toml_path,
            r#"
[project]
name = "test-project"
[scripts]
test = "echo ok"
"#,
        )
        .unwrap();

        let config = ProjectConfig::load(dir.path()).unwrap().unwrap();
        assert_eq!(config.project.name, "test-project");
        assert_eq!(config.scripts["test"], "echo ok");
    }

    #[test]
    fn test_load_missing_returns_none() {
        let dir = tempfile::TempDir::new().unwrap();
        let config = ProjectConfig::load(dir.path()).unwrap();
        assert!(config.is_none());
    }

    #[test]
    fn test_resolve_uses_repo_config_first() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("vg.toml"),
            r#"
[scripts]
test = "my-test"
"#,
        )
        .unwrap();

        let config = ProjectConfig::resolve(dir.path(), None);
        assert_eq!(config.scripts["test"], "my-test");
    }

    #[test]
    fn test_resolve_falls_back_to_workspace_defaults() {
        let root = tempfile::TempDir::new().unwrap();
        let repo = root.path().join("my-repo");
        std::fs::create_dir_all(&repo).unwrap();

        std::fs::write(
            root.path().join("vg.toml"),
            r#"
[workspace.defaults.scripts]
test = "workspace-test"
[workspace.defaults.watch]
run = ["test"]
"#,
        )
        .unwrap();

        let config = ProjectConfig::resolve(&repo, Some(root.path()));
        assert_eq!(config.scripts["test"], "workspace-test");
        assert_eq!(config.watch.run, vec!["test"]);
    }

    #[test]
    fn test_from_workspace_defaults_sets_name() {
        let defaults = WorkspaceDefaults {
            scripts: HashMap::from([("test".into(), "cargo test".into())]),
            ..Default::default()
        };
        let repo = std::path::PathBuf::from("/workspace/my-repo");
        let config = ProjectConfig::from_workspace_defaults(&defaults, &repo);
        assert_eq!(config.project.name, "my-repo");
        assert_eq!(config.scripts["test"], "cargo test");
    }

    #[test]
    fn test_has_watch_scripts() {
        let config = ProjectConfig {
            scripts: HashMap::from([("test".into(), "cargo test".into())]),
            watch: WatchSection {
                run: vec!["test".into()],
            },
            ..Default::default()
        };
        assert!(config.has_watch_scripts());

        let config_empty = ProjectConfig::default();
        assert!(!config_empty.has_watch_scripts());
    }
}
