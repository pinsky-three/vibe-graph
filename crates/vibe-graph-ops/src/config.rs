//! Configuration for the operations layer.

use std::path::PathBuf;

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

use crate::error::{OpsError, OpsResult};

/// Configuration for vibe-graph operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Maximum file size to include content (in KB).
    #[serde(default = "default_max_content_size_kb")]
    pub max_content_size_kb: u64,

    /// GitHub username for API authentication.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub github_username: Option<String>,

    /// GitHub token for API authentication.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub github_token: Option<String>,

    /// Global cache directory for cloned repositories.
    #[serde(default = "default_cache_dir")]
    pub cache_dir: PathBuf,
}

fn default_max_content_size_kb() -> u64 {
    100 // 100KB default
}

fn default_cache_dir() -> PathBuf {
    ProjectDirs::from("com", "pinsky-three", "vibe-graph")
        .map(|dirs| dirs.cache_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from(".vibe-graph-cache"))
}

impl Default for Config {
    fn default() -> Self {
        Self {
            max_content_size_kb: default_max_content_size_kb(),
            github_username: std::env::var("GITHUB_USERNAME").ok(),
            github_token: std::env::var("GITHUB_TOKEN").ok(),
            cache_dir: default_cache_dir(),
        }
    }
}

impl Config {
    /// Load configuration from disk with environment overrides.
    pub fn load() -> OpsResult<Self> {
        // Try to load from config file
        let config = if let Some(path) = Self::config_file_path() {
            if path.exists() {
                let contents = std::fs::read_to_string(&path)?;
                serde_json::from_str(&contents)?
            } else {
                Self::default()
            }
        } else {
            Self::default()
        };

        // Override with environment variables
        let config = Self {
            github_username: std::env::var("GITHUB_USERNAME")
                .ok()
                .or(config.github_username),
            github_token: std::env::var("GITHUB_TOKEN").ok().or(config.github_token),
            ..config
        };

        Ok(config)
    }

    /// Save configuration to disk.
    pub fn save(&self) -> OpsResult<()> {
        if let Some(path) = Self::config_file_path() {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let contents = serde_json::to_string_pretty(self)?;
            std::fs::write(&path, contents)?;
        }
        Ok(())
    }

    /// Get the path to the configuration file.
    pub fn config_file_path() -> Option<PathBuf> {
        ProjectDirs::from("com", "pinsky-three", "vibe-graph")
            .map(|dirs| dirs.config_dir().join("config.json"))
    }

    /// Check if GitHub credentials are configured.
    pub fn has_github(&self) -> bool {
        self.github_username.is_some() && self.github_token.is_some()
    }

    /// Validate that GitHub credentials are available.
    pub fn validate_github(&self) -> OpsResult<()> {
        if !self.has_github() {
            return Err(OpsError::GitHubNotConfigured);
        }
        Ok(())
    }

    /// Get the cache directory for a specific GitHub organization.
    pub fn org_cache_dir(&self, org: &str) -> PathBuf {
        self.cache_dir.join(org)
    }

    /// Get a configuration value by key.
    pub fn get(&self, key: &str) -> Option<String> {
        match key {
            "max_content_size_kb" => Some(self.max_content_size_kb.to_string()),
            "github_username" => self.github_username.clone(),
            "github_token" => self.github_token.as_ref().map(|_| "***".to_string()),
            "cache_dir" => Some(self.cache_dir.display().to_string()),
            _ => None,
        }
    }

    /// Set a configuration value by key.
    pub fn set(&mut self, key: &str, value: &str) -> OpsResult<()> {
        match key {
            "max_content_size_kb" => {
                self.max_content_size_kb = value
                    .parse()
                    .map_err(|_| OpsError::Config(format!("Invalid number: {}", value)))?;
            }
            "github_username" => {
                self.github_username = Some(value.to_string());
            }
            "github_token" => {
                self.github_token = Some(value.to_string());
            }
            "cache_dir" => {
                self.cache_dir = PathBuf::from(value);
            }
            _ => {
                return Err(OpsError::Config(format!("Unknown config key: {}", key)));
            }
        }
        Ok(())
    }
}

