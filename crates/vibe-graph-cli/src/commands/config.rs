//! Config command implementation.
//!
//! Manages CLI configuration.

use anyhow::Result;

use crate::config::Config;

/// Show current configuration.
pub fn show(config: &Config) -> Result<()> {
    println!("Vibe-Graph CLI Configuration");
    println!("{:-<40}", "");

    println!(
        "GitHub Token:        {}",
        config
            .github_token
            .as_ref()
            .map(|t| format!("{}...", &t[..8.min(t.len())]))
            .unwrap_or_else(|| "(not set)".to_string())
    );
    println!(
        "GitHub Username:     {}",
        config.github_username.as_deref().unwrap_or("(not set)")
    );
    println!(
        "GitHub Organization: {}",
        config.github_organization.as_deref().unwrap_or("(not set)")
    );
    println!("Cache Directory:     {}", config.cache_dir.display());
    println!("Output Directory:    {}", config.output_dir.display());
    println!("Max Content Size:    {} KB", config.max_content_size_kb);

    if let Some(config_path) = Config::config_file_path() {
        println!("\nConfig file: {}", config_path.display());
    }

    Ok(())
}

/// Set a configuration value.
pub fn set(config: &mut Config, key: &str, value: &str) -> Result<()> {
    match key {
        "github-token" | "token" => {
            config.github_token = Some(value.to_string());
            println!("✅ Set github-token");
            println!(
                "⚠️  Token stored in config file. For better security, use GITHUB_TOKEN env var."
            );
        }
        "github-username" | "username" => {
            config.github_username = Some(value.to_string());
            println!("Set github-username to: {}", value);
        }
        "github-organization" | "organization" | "org" => {
            config.github_organization = Some(value.to_string());
            println!("Set github-organization to: {}", value);
        }
        "max-content-size" | "max-size" => {
            config.max_content_size_kb = value.parse()?;
            println!("Set max-content-size to: {} KB", value);
        }
        _ => {
            anyhow::bail!(
                "Unknown config key: {}. Valid keys: github-token, github-username, github-organization, max-content-size",
                key
            );
        }
    }

    config.save()?;
    Ok(())
}

/// Get a configuration value.
pub fn get(config: &Config, key: &str) -> Result<()> {
    let value = match key {
        "github-token" | "token" => config
            .github_token
            .as_ref()
            .map(|t| format!("{}...", &t[..8.min(t.len())]))
            .unwrap_or_else(|| "(not set)".to_string()),
        "github-username" | "username" => config
            .github_username
            .clone()
            .unwrap_or_else(|| "(not set)".to_string()),
        "github-organization" | "organization" | "org" => config
            .github_organization
            .clone()
            .unwrap_or_else(|| "(not set)".to_string()),
        "cache-dir" => config.cache_dir.display().to_string(),
        "output-dir" => config.output_dir.display().to_string(),
        "max-content-size" | "max-size" => config.max_content_size_kb.to_string(),
        _ => {
            anyhow::bail!("Unknown config key: {}", key);
        }
    };

    println!("{}", value);
    Ok(())
}

/// Reset configuration to defaults.
pub fn reset() -> Result<()> {
    let config = Config::default();
    config.save()?;
    println!("Configuration reset to defaults");
    Ok(())
}
