//! File content endpoint.
//!
//! Serves file content from the workspace for the syntax-highlighted viewer.
//! Security: all paths are resolved relative to the workspace root and must
//! remain within it (directory traversal is rejected).

use std::path::PathBuf;
use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::types::ApiResponse;

/// Max file size we'll serve (1 MB).
const MAX_FILE_SIZE: u64 = 1_048_576;

/// Shared state for the file endpoint.
pub struct FileState {
    /// Workspace root — all file paths are resolved relative to this.
    pub workspace_root: PathBuf,
}

/// Query parameters for GET /file.
#[derive(Debug, Deserialize)]
pub struct FileQuery {
    /// Path to the file (absolute or relative to workspace root).
    pub path: String,
}

/// Successful file read response.
#[derive(Debug, Serialize)]
pub struct FileContentResponse {
    /// The file content as a UTF-8 string.
    pub content: String,
    /// Detected language identifier (for syntax highlighting).
    pub language: String,
    /// The resolved (canonical) path.
    pub path: String,
    /// Total number of lines.
    pub total_lines: usize,
    /// File size in bytes.
    pub size_bytes: u64,
}

/// Error response.
#[derive(Debug, Serialize)]
pub struct FileErrorResponse {
    pub code: String,
    pub message: String,
}

/// GET /file?path=... — Read file content for the viewer.
pub async fn file_handler(
    State(state): State<Arc<FileState>>,
    Query(query): Query<FileQuery>,
) -> impl IntoResponse {
    let requested = PathBuf::from(&query.path);

    // Resolve the path: try absolute first, then relative to workspace root
    let candidate = if requested.is_absolute() {
        requested.clone()
    } else {
        state.workspace_root.join(&requested)
    };

    // Canonicalize to resolve symlinks and `..` segments
    let resolved = match candidate.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            warn!(path = %query.path, error = %e, "file_handler: canonicalize failed");
            return (
                StatusCode::NOT_FOUND,
                Json(ApiResponse::new(FileErrorResponse {
                    code: "FILE_NOT_FOUND".into(),
                    message: format!("File not found: {}", query.path),
                })),
            )
                .into_response();
        }
    };

    // Security: ensure the resolved path is inside the workspace root
    let canonical_root = state
        .workspace_root
        .canonicalize()
        .unwrap_or_else(|_| state.workspace_root.clone());
    if !resolved.starts_with(&canonical_root) {
        warn!(
            path = %resolved.display(),
            root = %canonical_root.display(),
            "file_handler: path traversal rejected"
        );
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse::new(FileErrorResponse {
                code: "PATH_TRAVERSAL".into(),
                message: "Path is outside workspace root".into(),
            })),
        )
            .into_response();
    }

    // Check file exists and is a file
    if !resolved.is_file() {
        return (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::new(FileErrorResponse {
                code: "NOT_A_FILE".into(),
                message: format!("Not a file: {}", resolved.display()),
            })),
        )
            .into_response();
    }

    // Check file size
    let metadata = match std::fs::metadata(&resolved) {
        Ok(m) => m,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse::new(FileErrorResponse {
                    code: "IO_ERROR".into(),
                    message: format!("Could not stat file: {}", e),
                })),
            )
                .into_response();
        }
    };

    let size_bytes = metadata.len();
    if size_bytes > MAX_FILE_SIZE {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(ApiResponse::new(FileErrorResponse {
                code: "FILE_TOO_LARGE".into(),
                message: format!(
                    "File is {} bytes (max {} bytes)",
                    size_bytes, MAX_FILE_SIZE
                ),
            })),
        )
            .into_response();
    }

    // Read as UTF-8
    let content = match std::fs::read_to_string(&resolved) {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(ApiResponse::new(FileErrorResponse {
                    code: "NOT_TEXT".into(),
                    message: format!("Could not read as text: {}", e),
                })),
            )
                .into_response();
        }
    };

    let total_lines = content.lines().count();
    let language = detect_language(&resolved);

    info!(
        path = %resolved.display(),
        lines = total_lines,
        size = size_bytes,
        lang = language,
        "file_handler: served"
    );

    (
        StatusCode::OK,
        Json(ApiResponse::new(FileContentResponse {
            content,
            language: language.to_string(),
            path: resolved.display().to_string(),
            total_lines,
            size_bytes,
        })),
    )
        .into_response()
}

// =============================================================================
// Language detection (shared with viz crate; duplicated here to avoid dep)
// =============================================================================

fn detect_language(path: &std::path::Path) -> &'static str {
    // Check special filenames
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        let lower = name.to_lowercase();
        match lower.as_str() {
            "dockerfile" => return "dockerfile",
            "makefile" | "gnumakefile" => return "sh",
            "cargo.toml" | "cargo.lock" => return "toml",
            ".gitignore" | ".dockerignore" => return "txt",
            _ => {}
        }
    }

    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| match ext.to_lowercase().as_str() {
            "rs" => "rs",
            "py" | "pyi" => "py",
            "md" | "markdown" => "md",
            "toml" => "toml",
            "ts" | "tsx" => "ts",
            "js" | "jsx" | "mjs" | "cjs" => "js",
            "json" => "json",
            "yaml" | "yml" => "yaml",
            "html" | "htm" => "html",
            "css" | "scss" | "sass" => "css",
            "sh" | "bash" | "zsh" => "sh",
            "sql" => "sql",
            "c" | "h" => "c",
            "cpp" | "cxx" | "cc" | "hpp" => "cpp",
            "go" => "go",
            "java" => "java",
            "xml" | "svg" => "xml",
            _ => "txt",
        })
        .unwrap_or("txt")
}
