//! Embedded frontend assets for self-contained binary distribution.
//!
//! When built with the `embedded-frontend` feature, this module embeds
//! all files from `frontend/dist/` into the binary at compile time.

use axum::{
    body::Body,
    http::{Request, Response, StatusCode},
    response::IntoResponse,
};

#[cfg(feature = "embedded-frontend")]
use axum::http::header;
#[cfg(feature = "embedded-frontend")]
use rust_embed::RustEmbed;
#[cfg(feature = "embedded-frontend")]
use std::borrow::Cow;

/// Embedded frontend assets from `frontend/dist/`.
///
/// This struct embeds all frontend files at compile time when the
/// `embedded-frontend` feature is enabled.
#[cfg(feature = "embedded-frontend")]
#[derive(RustEmbed)]
#[folder = "../../frontend/dist/"]
#[prefix = ""]
pub struct FrontendAssets;

/// Check if embedded frontend is available.
#[cfg(feature = "embedded-frontend")]
pub fn has_embedded_frontend() -> bool {
    // Check if index.html exists in embedded assets
    FrontendAssets::get("index.html").is_some()
}

#[cfg(not(feature = "embedded-frontend"))]
pub fn has_embedded_frontend() -> bool {
    false
}

/// Serve an embedded frontend file.
#[cfg(feature = "embedded-frontend")]
pub async fn serve_embedded(req: Request<Body>) -> impl IntoResponse {
    let path = req.uri().path().trim_start_matches('/');

    // Default to index.html for root or SPA routes
    let path = if path.is_empty() || !path.contains('.') {
        "index.html"
    } else {
        path
    };

    match FrontendAssets::get(path) {
        Some(content) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            let body: Cow<'static, [u8]> = content.data;

            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, mime.as_ref())
                .header(header::CACHE_CONTROL, cache_control_for(path))
                .body(Body::from(body.into_owned()))
                .unwrap()
        }
        None => {
            // Try index.html for SPA fallback
            if let Some(content) = FrontendAssets::get("index.html") {
                let body: Cow<'static, [u8]> = content.data;
                Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_TYPE, "text/html")
                    .body(Body::from(body.into_owned()))
                    .unwrap()
            } else {
                Response::builder()
                    .status(StatusCode::NOT_FOUND)
                    .body(Body::from("Not Found"))
                    .unwrap()
            }
        }
    }
}

#[cfg(not(feature = "embedded-frontend"))]
pub async fn serve_embedded(_req: Request<Body>) -> impl IntoResponse {
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .body(Body::from("Embedded frontend not available"))
        .unwrap()
}

/// Determine cache control header based on file type.
#[cfg(feature = "embedded-frontend")]
fn cache_control_for(path: &str) -> &'static str {
    if path.starts_with("assets/") {
        // Hashed assets can be cached forever
        "public, max-age=31536000, immutable"
    } else if path == "index.html" {
        // HTML should be revalidated
        "no-cache"
    } else {
        // Other files: cache for a short time
        "public, max-age=3600"
    }
}

/// List all embedded frontend files (for debugging).
#[cfg(feature = "embedded-frontend")]
#[allow(dead_code)]
pub fn list_embedded_files() -> Vec<String> {
    FrontendAssets::iter().map(|s| s.to_string()).collect()
}

#[cfg(not(feature = "embedded-frontend"))]
#[allow(dead_code)]
pub fn list_embedded_files() -> Vec<String> {
    vec![]
}
