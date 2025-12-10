//! Compose command implementation.
//!
//! Generates composed output (markdown, JSON) from projects.

use std::path::PathBuf;

use anyhow::{Context, Result};
use askama::Template;
use tracing::info;

use crate::config::Config;
use crate::project::Project;

/// Output format for composition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Markdown,
    Json,
}

impl std::str::FromStr for OutputFormat {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "md" | "markdown" => Ok(Self::Markdown),
            "json" => Ok(Self::Json),
            _ => anyhow::bail!("Unknown format: {}. Use 'md' or 'json'", s),
        }
    }
}

/// Compose a project to the specified output.
pub fn execute(
    config: &Config,
    project: &mut Project,
    output: Option<PathBuf>,
    format: OutputFormat,
) -> Result<()> {
    info!(
        project = %project.name,
        repos = project.repositories.len(),
        "Composing project"
    );

    // Expand content for small text files
    let max_size = config.max_content_size_kb * 1024;
    project.expand_content(|source| {
        source.size.map(|s| s < max_size).unwrap_or(false) && source.is_text()
    })?;

    let output_path = output.unwrap_or_else(|| {
        let ext = match format {
            OutputFormat::Markdown => "md",
            OutputFormat::Json => "json",
        };
        config.output_dir.join(format!("{}.{}", project.name, ext))
    });

    let content = match format {
        OutputFormat::Markdown => render_markdown(project)?,
        OutputFormat::Json => render_json(project)?,
    };

    // Ensure parent directory exists
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create output directory: {}", parent.display()))?;
    }

    std::fs::write(&output_path, content)
        .with_context(|| format!("Failed to write output to: {}", output_path.display()))?;

    println!("Output written to: {}", output_path.display());
    println!(
        "  {} repositories, {} files",
        project.repositories.len(),
        project.total_sources()
    );

    Ok(())
}

/// Render project as markdown.
fn render_markdown(project: &Project) -> Result<String> {
    let template = ComposerTemplate { project };
    template
        .render()
        .with_context(|| "Failed to render markdown template")
}

/// Render project as JSON.
fn render_json(project: &Project) -> Result<String> {
    serde_json::to_string_pretty(project).with_context(|| "Failed to serialize project to JSON")
}

/// Askama template for markdown output.
#[derive(Template)]
#[template(path = "composer.md", escape = "none")]
struct ComposerTemplate<'a> {
    project: &'a Project,
}
