//! Project type inference from filesystem markers.
//!
//! Detects the project type (Rust, Node, Python, Go, etc.) by checking
//! for well-known marker files, then generates default scripts and a
//! `ProjectConfig` for projects without an explicit `vg.toml`.

use std::collections::HashMap;
use std::path::Path;

use tracing::debug;

use crate::project_config::{ProjectConfig, ProjectSection, WatchSection};

/// Detected project type based on filesystem markers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectType {
    Rust,
    Node,
    Python,
    Go,
    Make,
    Docker,
    Unknown,
}

impl std::fmt::Display for ProjectType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Rust => write!(f, "Rust"),
            Self::Node => write!(f, "Node"),
            Self::Python => write!(f, "Python"),
            Self::Go => write!(f, "Go"),
            Self::Make => write!(f, "Make"),
            Self::Docker => write!(f, "Docker"),
            Self::Unknown => write!(f, "Unknown"),
        }
    }
}

/// Detect the project type by checking for marker files in the given directory.
///
/// Checks markers in priority order (most specific first). Returns the first match.
pub fn detect_project_type(path: &Path) -> ProjectType {
    if path.join("Cargo.toml").exists() {
        ProjectType::Rust
    } else if path.join("package.json").exists() {
        ProjectType::Node
    } else if path.join("pyproject.toml").exists() || path.join("setup.py").exists() {
        ProjectType::Python
    } else if path.join("go.mod").exists() {
        ProjectType::Go
    } else if path.join("Makefile").exists() || path.join("makefile").exists() {
        ProjectType::Make
    } else if path.join("docker-compose.yml").exists() || path.join("docker-compose.yaml").exists()
    {
        ProjectType::Docker
    } else {
        ProjectType::Unknown
    }
}

/// Generate default scripts for a given project type.
pub fn infer_scripts(project_type: &ProjectType) -> HashMap<String, String> {
    let mut scripts = HashMap::new();
    match project_type {
        ProjectType::Rust => {
            scripts.insert("build".into(), "cargo build".into());
            scripts.insert("test".into(), "cargo test".into());
            scripts.insert("lint".into(), "cargo clippy -- -D warnings".into());
            scripts.insert("check".into(), "cargo check".into());
        }
        ProjectType::Node => {
            scripts.insert("build".into(), "npm run build".into());
            scripts.insert("test".into(), "npm test".into());
            scripts.insert("lint".into(), "npm run lint".into());
            scripts.insert("dev".into(), "npm run dev".into());
        }
        ProjectType::Python => {
            scripts.insert("test".into(), "pytest".into());
            scripts.insert("lint".into(), "ruff check .".into());
            scripts.insert("check".into(), "python -m py_compile".into());
        }
        ProjectType::Go => {
            scripts.insert("build".into(), "go build ./...".into());
            scripts.insert("test".into(), "go test ./...".into());
            scripts.insert("lint".into(), "golangci-lint run".into());
        }
        ProjectType::Make => {
            // We can't know the targets without parsing the Makefile,
            // so we provide common conventions
            scripts.insert("build".into(), "make build".into());
            scripts.insert("test".into(), "make test".into());
            scripts.insert("lint".into(), "make lint".into());
        }
        ProjectType::Docker => {
            scripts.insert("dev".into(), "docker compose up".into());
            scripts.insert("build".into(), "docker compose build".into());
        }
        ProjectType::Unknown => {}
    }
    scripts
}

/// Infer default watch scripts for a project type.
fn infer_watch_scripts(project_type: &ProjectType) -> Vec<String> {
    match project_type {
        ProjectType::Rust => vec!["check".into(), "test".into()],
        ProjectType::Node => vec!["test".into()],
        ProjectType::Python => vec!["test".into()],
        ProjectType::Go => vec!["test".into()],
        ProjectType::Make => vec!["test".into()],
        ProjectType::Docker | ProjectType::Unknown => vec![],
    }
}

/// Infer a full `ProjectConfig` from the filesystem.
///
/// Detects the project type and generates appropriate default scripts
/// and watch configuration.
pub fn infer_config(path: &Path) -> ProjectConfig {
    let project_type = detect_project_type(path);
    let scripts = infer_scripts(&project_type);
    let watch_run = infer_watch_scripts(&project_type);

    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();

    debug!(
        path = %path.display(),
        project_type = %project_type,
        scripts = scripts.len(),
        "Inferred project config"
    );

    // For Node projects, try to read package.json scripts
    let scripts = if project_type == ProjectType::Node {
        read_package_json_scripts(path).unwrap_or(scripts)
    } else {
        scripts
    };

    ProjectConfig {
        project: ProjectSection { name },
        scripts,
        watch: WatchSection { run: watch_run },
        ..Default::default()
    }
}

/// Try to read scripts from a Node.js `package.json`.
///
/// Maps well-known npm script names to vg script names.
fn read_package_json_scripts(path: &Path) -> Option<HashMap<String, String>> {
    let pkg_path = path.join("package.json");
    let content = std::fs::read_to_string(pkg_path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&content).ok()?;
    let pkg_scripts = value.get("scripts")?.as_object()?;

    let mut scripts = HashMap::new();
    let well_known = ["build", "test", "lint", "dev", "start", "check"];

    for name in &well_known {
        if let Some(cmd) = pkg_scripts.get(*name).and_then(|v| v.as_str()) {
            scripts.insert(name.to_string(), format!("npm run {}", name));
            debug!(script = name, cmd = cmd, "Read npm script");
        }
    }

    if scripts.is_empty() {
        None
    } else {
        Some(scripts)
    }
}

/// Generate a TOML string from a `ProjectConfig`, suitable for writing to `vg.toml`.
pub fn generate_toml(config: &ProjectConfig) -> String {
    let mut out = String::new();

    // [project]
    out.push_str("[project]\n");
    out.push_str(&format!("name = \"{}\"\n", config.project.name));
    out.push('\n');

    // [scripts]
    if !config.scripts.is_empty() {
        out.push_str("[scripts]\n");
        let mut sorted: Vec<_> = config.scripts.iter().collect();
        sorted.sort_by_key(|(k, _)| *k);
        for (name, cmd) in sorted {
            out.push_str(&format!("{} = \"{}\"\n", name, cmd));
        }
        out.push('\n');
    }

    // [watch]
    if !config.watch.run.is_empty() {
        out.push_str("[watch]\n");
        let items: Vec<String> = config.watch.run.iter().map(|s| format!("\"{}\"", s)).collect();
        out.push_str(&format!("run = [{}]\n", items.join(", ")));
        out.push('\n');
    }

    // [stability]
    if !config.stability.is_empty() {
        out.push_str("[stability]\n");
        let mut sorted: Vec<_> = config.stability.iter().collect();
        sorted.sort_by_key(|(k, _)| (*k).clone());
        for (role, target) in sorted {
            out.push_str(&format!("{} = {:.2}\n", role, target));
        }
        out.push('\n');
    }

    // [ignore]
    if !config.ignore.directories.is_empty() || !config.ignore.patterns.is_empty() {
        out.push_str("[ignore]\n");
        if !config.ignore.directories.is_empty() {
            let items: Vec<String> = config
                .ignore
                .directories
                .iter()
                .map(|s| format!("\"{}\"", s))
                .collect();
            out.push_str(&format!("directories = [{}]\n", items.join(", ")));
        }
        if !config.ignore.patterns.is_empty() {
            let items: Vec<String> = config
                .ignore
                .patterns
                .iter()
                .map(|s| format!("\"{}\"", s))
                .collect();
            out.push_str(&format!("patterns = [{}]\n", items.join(", ")));
        }
        out.push('\n');
    }

    // [workspace] (if present)
    if let Some(ws) = &config.workspace {
        if !ws.defaults.scripts.is_empty() {
            out.push_str("[workspace.defaults.scripts]\n");
            let mut sorted: Vec<_> = ws.defaults.scripts.iter().collect();
            sorted.sort_by_key(|(k, _)| *k);
            for (name, cmd) in sorted {
                out.push_str(&format!("{} = \"{}\"\n", name, cmd));
            }
            out.push('\n');
        }
        if !ws.defaults.watch.run.is_empty() {
            out.push_str("[workspace.defaults.watch]\n");
            let items: Vec<String> = ws
                .defaults
                .watch
                .run
                .iter()
                .map(|s| format!("\"{}\"", s))
                .collect();
            out.push_str(&format!("run = [{}]\n", items.join(", ")));
            out.push('\n');
        }
    }

    out
}

/// Generate a workspace-style TOML config for a multi-repo root.
pub fn generate_workspace_toml(name: &str, project_type: &ProjectType) -> String {
    let scripts = infer_scripts(project_type);
    let watch_run = infer_watch_scripts(project_type);

    let config = ProjectConfig {
        project: ProjectSection {
            name: name.to_string(),
        },
        workspace: Some(crate::project_config::WorkspaceSection {
            defaults: crate::project_config::WorkspaceDefaults {
                scripts,
                watch: WatchSection { run: watch_run },
                ..Default::default()
            },
        }),
        ..Default::default()
    };

    generate_toml(&config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_rust_project() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();
        assert_eq!(detect_project_type(dir.path()), ProjectType::Rust);
    }

    #[test]
    fn test_detect_node_project() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("package.json"), "{}").unwrap();
        assert_eq!(detect_project_type(dir.path()), ProjectType::Node);
    }

    #[test]
    fn test_detect_python_project() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("pyproject.toml"), "").unwrap();
        assert_eq!(detect_project_type(dir.path()), ProjectType::Python);
    }

    #[test]
    fn test_detect_go_project() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("go.mod"), "module example").unwrap();
        assert_eq!(detect_project_type(dir.path()), ProjectType::Go);
    }

    #[test]
    fn test_detect_make_project() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("Makefile"), "all:").unwrap();
        assert_eq!(detect_project_type(dir.path()), ProjectType::Make);
    }

    #[test]
    fn test_detect_docker_project() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("docker-compose.yml"), "version: '3'").unwrap();
        assert_eq!(detect_project_type(dir.path()), ProjectType::Docker);
    }

    #[test]
    fn test_detect_unknown_project() {
        let dir = tempfile::TempDir::new().unwrap();
        assert_eq!(detect_project_type(dir.path()), ProjectType::Unknown);
    }

    #[test]
    fn test_rust_precedence_over_make() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();
        std::fs::write(dir.path().join("Makefile"), "all:").unwrap();
        // Rust should win since it's checked first
        assert_eq!(detect_project_type(dir.path()), ProjectType::Rust);
    }

    #[test]
    fn test_infer_rust_scripts() {
        let scripts = infer_scripts(&ProjectType::Rust);
        assert_eq!(scripts["build"], "cargo build");
        assert_eq!(scripts["test"], "cargo test");
        assert_eq!(scripts["lint"], "cargo clippy -- -D warnings");
        assert_eq!(scripts["check"], "cargo check");
    }

    #[test]
    fn test_infer_config_sets_name() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();
        let config = infer_config(dir.path());
        assert!(!config.project.name.is_empty());
        assert!(config.has_scripts());
    }

    #[test]
    fn test_generate_toml_roundtrip() {
        let config = ProjectConfig {
            project: ProjectSection {
                name: "test-project".into(),
            },
            scripts: HashMap::from([
                ("build".into(), "cargo build".into()),
                ("test".into(), "cargo test".into()),
            ]),
            watch: WatchSection {
                run: vec!["test".into()],
            },
            ..Default::default()
        };

        let toml_str = generate_toml(&config);
        assert!(toml_str.contains("[project]"));
        assert!(toml_str.contains("name = \"test-project\""));
        assert!(toml_str.contains("[scripts]"));
        assert!(toml_str.contains("test = \"cargo test\""));
        assert!(toml_str.contains("[watch]"));
        assert!(toml_str.contains("run = [\"test\"]"));
    }

    #[test]
    fn test_read_package_json_scripts() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"scripts": {"test": "jest", "build": "tsc", "lint": "eslint .", "dev": "vite"}}"#,
        )
        .unwrap();
        let scripts = read_package_json_scripts(dir.path()).unwrap();
        assert!(scripts.contains_key("test"));
        assert!(scripts.contains_key("build"));
        assert!(scripts.contains_key("lint"));
        assert!(scripts.contains_key("dev"));
    }

    #[test]
    fn test_generate_workspace_toml() {
        let toml_str = generate_workspace_toml("my-org", &ProjectType::Rust);
        assert!(toml_str.contains("[workspace.defaults.scripts]"));
        assert!(toml_str.contains("test = \"cargo test\""));
        assert!(toml_str.contains("[workspace.defaults.watch]"));
    }
}
