//! CLI configuration management.
//!
//! Supports loading configuration from environment variables, config files,
//! and CLI arguments with proper precedence.

use std::path::PathBuf;

use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

/// Application-wide configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// GitHub personal access token for API access.
    #[serde(skip_serializing)]
    pub github_token: Option<String>,

    /// GitHub username for authentication.
    pub github_username: Option<String>,

    /// Default GitHub organization to operate on.
    pub github_organization: Option<String>,

    /// Directory where repositories are cloned/cached.
    pub cache_dir: PathBuf,

    /// Default output directory for composed files.
    pub output_dir: PathBuf,

    /// Maximum file size (in KB) to include content in output.
    pub max_content_size_kb: u64,
}

impl Default for Config {
    fn default() -> Self {
        let cache_dir = ProjectDirs::from("dev", "vibe-graph", "vg")
            .map(|dirs| dirs.cache_dir().to_path_buf())
            .unwrap_or_else(|| std::env::temp_dir().join("vibe-graph"));

        Self {
            github_token: None,
            github_username: None,
            github_organization: None,
            cache_dir,
            output_dir: PathBuf::from("."),
            max_content_size_kb: 50,
        }
    }
}

impl Config {
    /// Load configuration from environment variables and config file.
    pub fn load() -> Result<Self> {
        // Load .env file if present (silently ignore if missing)
        let _ = dotenvy::dotenv();

        let mut config = Self::default();

        // Override with environment variables
        if let Ok(token) = std::env::var("GITHUB_TOKEN") {
            config.github_token = Some(token);
        }
        if let Ok(username) = std::env::var("GITHUB_USERNAME") {
            config.github_username = Some(username);
        }
        if let Ok(org) = std::env::var("GITHUB_ORGANIZATION") {
            config.github_organization = Some(org);
        }
        if let Ok(cache_dir) = std::env::var("VG_CACHE_DIR") {
            config.cache_dir = PathBuf::from(cache_dir);
        }
        if let Ok(output_dir) = std::env::var("VG_OUTPUT_DIR") {
            config.output_dir = PathBuf::from(output_dir);
        }
        if let Ok(max_size) = std::env::var("VG_MAX_CONTENT_SIZE_KB") {
            config.max_content_size_kb = max_size.parse().unwrap_or(50);
        }

        // Try to load from config file
        if let Some(config_path) = Self::config_file_path() {
            if config_path.exists() {
                let contents = std::fs::read_to_string(&config_path).with_context(|| {
                    format!("Failed to read config from {}", config_path.display())
                })?;
                let file_config: Config = serde_json::from_str(&contents)
                    .with_context(|| "Failed to parse config file")?;

                // File config takes lower precedence than env vars
                if config.github_token.is_none() {
                    config.github_token = file_config.github_token;
                }
                if config.github_username.is_none() {
                    config.github_username = file_config.github_username;
                }
                if config.github_organization.is_none() {
                    config.github_organization = file_config.github_organization;
                }
            }
        }

        Ok(config)
    }

    /// Save current configuration to the config file.
    pub fn save(&self) -> Result<()> {
        if let Some(config_path) = Self::config_file_path() {
            if let Some(parent) = config_path.parent() {
                std::fs::create_dir_all(parent).with_context(|| {
                    format!("Failed to create config directory: {}", parent.display())
                })?;
            }
            let contents = serde_json::to_string_pretty(self)?;
            std::fs::write(&config_path, contents)
                .with_context(|| format!("Failed to write config to {}", config_path.display()))?;
        }
        Ok(())
    }

    /// Get the path to the config file.
    pub fn config_file_path() -> Option<PathBuf> {
        ProjectDirs::from("dev", "vibe-graph", "vg")
            .map(|dirs| dirs.config_dir().join("config.json"))
    }

    /// Validate that required configuration is present for GitHub operations.
    pub fn validate_github(&self) -> Result<()> {
        if self.github_token.is_none() {
            anyhow::bail!(
                "GitHub token required. Set GITHUB_TOKEN environment variable or run `vg config set github-token <token>`"
            );
        }
        if self.github_username.is_none() {
            anyhow::bail!(
                "GitHub username required. Set GITHUB_USERNAME environment variable or run `vg config set github-username <username>`"
            );
        }
        Ok(())
    }

    /// Get the cache directory for a specific organization.
    pub fn org_cache_dir(&self, org: &str) -> PathBuf {
        self.cache_dir.join("orgs").join(org)
    }

    /// Get the cache directory for a specific repository.
    #[allow(dead_code)]
    pub fn repo_cache_dir(&self, org: &str, repo: &str) -> PathBuf {
        self.org_cache_dir(org).join(repo)
    }
}
