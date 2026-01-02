//! Integration tests for the Vibe-Graph REST API.
//!
//! These tests verify the API endpoints work correctly and return
//! the expected responses.
//!
//! Run with: `cargo test --package vibe-graph-api --test api_integration`

use std::fs;
use std::path::Path;

use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
    Router,
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tempfile::TempDir;
use tower::ServiceExt;
use vibe_graph_ops::{Config, OpsContext};

use vibe_graph_api::create_ops_router;

/// Create a test router with a fresh OpsContext.
fn create_test_router() -> Router {
    let config = Config::default();
    let ctx = OpsContext::new(config);
    create_ops_router(ctx)
}

/// Create a test router with a specific config.
#[allow(dead_code)]
fn create_test_router_with_config(config: Config) -> Router {
    let ctx = OpsContext::new(config);
    create_ops_router(ctx)
}

/// Helper to make a GET request.
async fn get(router: &Router, uri: &str) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(Method::GET)
        .uri(uri)
        .body(Body::empty())
        .unwrap();

    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap_or(json!(null));

    (status, json)
}

/// Helper to make a POST request with JSON body.
async fn post(router: &Router, uri: &str, body: Value) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(Method::POST)
        .uri(uri)
        .header("Content-Type", "application/json")
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap();

    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap_or(json!(null));

    (status, json)
}

/// Helper to make a DELETE request.
async fn delete(router: &Router, uri: &str) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(Method::DELETE)
        .uri(uri)
        .body(Body::empty())
        .unwrap();

    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap_or(json!(null));

    (status, json)
}

/// Create a minimal git repository for testing.
fn create_test_repo(dir: &Path) {
    // Create .git directory (minimal git repo marker)
    fs::create_dir_all(dir.join(".git")).unwrap();

    // Create some source files
    fs::write(
        dir.join("main.rs"),
        r#"
fn main() {
    println!("Hello, world!");
}
"#,
    )
    .unwrap();

    fs::write(
        dir.join("lib.rs"),
        r#"
pub fn hello() -> &'static str {
    "Hello"
}
"#,
    )
    .unwrap();

    // Create a subdirectory with more files
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(
        dir.join("src/utils.rs"),
        r#"
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}
"#,
    )
    .unwrap();
}

/// Create a multi-repo workspace for testing.
fn create_multi_repo_workspace(dir: &Path) {
    let repo1 = dir.join("repo1");
    let repo2 = dir.join("repo2");

    fs::create_dir_all(&repo1).unwrap();
    fs::create_dir_all(&repo2).unwrap();

    create_test_repo(&repo1);
    create_test_repo(&repo2);
}

// =============================================================================
// Sync Endpoint Tests
// =============================================================================

#[tokio::test]
async fn test_sync_post_local_path() {
    let temp = TempDir::new().unwrap();
    let test_dir = temp.path();
    create_test_repo(test_dir);

    let router = create_test_router();

    let (status, json) = post(
        &router,
        "/sync",
        json!({
            "source": { "type": "local", "path": test_dir.to_str().unwrap() },
            "ignore": [],
            "no_save": false,
            "snapshot": false,
            "use_cache": false,
            "force": false
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "Sync should succeed: {:?}", json);
    assert!(
        json.get("data").is_some(),
        "Response should have data field"
    );

    let data = &json["data"];
    assert!(
        data.get("project").is_some(),
        "Response should have project"
    );
    assert!(
        data.get("workspace").is_some(),
        "Response should have workspace"
    );

    // Verify .self folder was created
    assert!(test_dir.join(".self").exists(), ".self should be created");
}

#[tokio::test]
async fn test_sync_get_query_params() {
    let temp = TempDir::new().unwrap();
    let test_dir = temp.path();
    create_test_repo(test_dir);

    let router = create_test_router();

    let uri = format!("/sync?source={}", test_dir.to_str().unwrap());
    let (status, json) = get(&router, &uri).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "Sync GET should succeed: {:?}",
        json
    );
    assert!(
        json.get("data").is_some(),
        "Response should have data field"
    );
}

#[tokio::test]
async fn test_sync_no_save_option() {
    let temp = TempDir::new().unwrap();
    let test_dir = temp.path();
    create_test_repo(test_dir);

    let router = create_test_router();

    let (status, json) = post(
        &router,
        "/sync",
        json!({
            "source": { "type": "local", "path": test_dir.to_str().unwrap() },
            "ignore": [],
            "no_save": true,
            "snapshot": false,
            "use_cache": false,
            "force": false
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "Sync should succeed: {:?}", json);

    // .self folder should NOT be created with no_save
    assert!(
        !test_dir.join(".self").exists(),
        ".self should NOT be created with no_save=true"
    );
}

#[tokio::test]
async fn test_sync_response_structure() {
    let temp = TempDir::new().unwrap();
    let test_dir = temp.path();
    create_test_repo(test_dir);

    let router = create_test_router();

    let (status, json) = post(
        &router,
        "/sync",
        json!({
            "source": { "type": "local", "path": test_dir.to_str().unwrap() },
            "ignore": [],
            "no_save": false,
            "snapshot": false,
            "use_cache": false,
            "force": false
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);

    let data = &json["data"];

    // Verify project structure
    let project = &data["project"];
    assert!(project["name"].is_string(), "Project should have name");
    assert!(
        project["repositories"].is_array(),
        "Project should have repositories"
    );

    // Verify workspace structure
    let workspace = &data["workspace"];
    assert!(workspace["name"].is_string(), "Workspace should have name");
    assert!(
        workspace["root"].is_string(),
        "Workspace should have root path"
    );
    // kind is an object with "type" field due to serde tagging
    assert!(workspace["kind"].is_object(), "Workspace should have kind");
    assert!(
        workspace["kind"]["type"].is_string(),
        "Workspace kind should have type"
    );
}

#[tokio::test]
async fn test_sync_multi_repo_workspace() {
    let temp = TempDir::new().unwrap();
    let test_dir = temp.path();
    create_multi_repo_workspace(test_dir);

    let router = create_test_router();

    let (status, json) = post(
        &router,
        "/sync",
        json!({
            "source": { "type": "local", "path": test_dir.to_str().unwrap() },
            "ignore": [],
            "no_save": false,
            "snapshot": false,
            "use_cache": false,
            "force": false
        }),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "Multi-repo sync should succeed: {:?}",
        json
    );

    let repos = &json["data"]["project"]["repositories"];
    assert!(repos.is_array(), "Should have repositories array");
    assert_eq!(repos.as_array().unwrap().len(), 2, "Should have 2 repos");
}

// =============================================================================
// Status Endpoint Tests
// =============================================================================

#[tokio::test]
async fn test_status_unsynced_workspace() {
    let temp = TempDir::new().unwrap();
    let test_dir = temp.path();
    create_test_repo(test_dir);

    let router = create_test_router();

    let uri = format!("/status?path={}", test_dir.to_str().unwrap());
    let (status, json) = get(&router, &uri).await;

    assert_eq!(status, StatusCode::OK, "Status should succeed: {:?}", json);

    let data = &json["data"];
    assert!(data["workspace"].is_object(), "Should have workspace info");
    assert_eq!(
        data["store_exists"].as_bool(),
        Some(false),
        "Store should not exist"
    );
}

#[tokio::test]
async fn test_status_synced_workspace() {
    let temp = TempDir::new().unwrap();
    let test_dir = temp.path();
    create_test_repo(test_dir);

    let router = create_test_router();

    // First sync
    let (status, _) = post(
        &router,
        "/sync",
        json!({
            "source": { "type": "local", "path": test_dir.to_str().unwrap() },
            "ignore": [],
            "no_save": false,
            "snapshot": false,
            "use_cache": false,
            "force": false
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Then check status
    let uri = format!("/status?path={}", test_dir.to_str().unwrap());
    let (status, json) = get(&router, &uri).await;

    assert_eq!(status, StatusCode::OK, "Status should succeed: {:?}", json);

    let data = &json["data"];
    assert!(data["workspace"].is_object(), "Should have workspace info");
    assert_eq!(
        data["store_exists"].as_bool(),
        Some(true),
        "Store should exist after sync"
    );
    assert!(
        data["manifest"].is_object(),
        "Should have manifest after sync"
    );
}

#[tokio::test]
async fn test_status_detailed() {
    let temp = TempDir::new().unwrap();
    let test_dir = temp.path();
    create_test_repo(test_dir);

    let router = create_test_router();

    // Sync first
    post(
        &router,
        "/sync",
        json!({
            "source": { "type": "local", "path": test_dir.to_str().unwrap() },
            "ignore": [],
            "no_save": false,
            "snapshot": false,
            "use_cache": false,
            "force": false
        }),
    )
    .await;

    let uri = format!("/status?path={}&detailed=true", test_dir.to_str().unwrap());
    let (status, json) = get(&router, &uri).await;

    assert_eq!(status, StatusCode::OK, "Status should succeed: {:?}", json);
}

// =============================================================================
// Graph Endpoint Tests
// =============================================================================

#[tokio::test]
async fn test_graph_after_sync() {
    let temp = TempDir::new().unwrap();
    let test_dir = temp.path();
    create_test_repo(test_dir);

    let router = create_test_router();

    // First sync
    let (status, _) = post(
        &router,
        "/sync",
        json!({
            "source": { "type": "local", "path": test_dir.to_str().unwrap() },
            "ignore": [],
            "no_save": false,
            "snapshot": false,
            "use_cache": false,
            "force": false
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Then build graph
    let (status, json) = post(
        &router,
        "/graph",
        json!({
            "path": test_dir.to_str().unwrap(),
            "output": null,
            "force": false
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "Graph should succeed: {:?}", json);

    let data = &json["data"];
    assert!(data["graph"].is_object(), "Should have graph");
    assert!(data["graph"]["nodes"].is_array(), "Graph should have nodes");
    assert!(data["graph"]["edges"].is_array(), "Graph should have edges");

    // Verify graph.json was created
    assert!(
        test_dir.join(".self/graph.json").exists(),
        "graph.json should be created"
    );
}

#[tokio::test]
async fn test_graph_get_query() {
    let temp = TempDir::new().unwrap();
    let test_dir = temp.path();
    create_test_repo(test_dir);

    let router = create_test_router();

    // First sync
    post(
        &router,
        "/sync",
        json!({
            "source": { "type": "local", "path": test_dir.to_str().unwrap() },
            "ignore": [],
            "no_save": false,
            "snapshot": false,
            "use_cache": false,
            "force": false
        }),
    )
    .await;

    // Then GET graph
    let uri = format!("/graph?path={}", test_dir.to_str().unwrap());
    let (status, json) = get(&router, &uri).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "Graph GET should succeed: {:?}",
        json
    );

    let data = &json["data"];
    assert!(data["graph"].is_object(), "Should have graph");
}

#[tokio::test]
async fn test_graph_fails_without_sync() {
    let temp = TempDir::new().unwrap();
    let test_dir = temp.path();
    create_test_repo(test_dir);

    let router = create_test_router();

    // Graph without sync should fail
    let uri = format!("/graph?path={}", test_dir.to_str().unwrap());
    let (status, json) = get(&router, &uri).await;

    assert_eq!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "Graph should fail without sync: {:?}",
        json
    );
}

#[tokio::test]
async fn test_graph_response_structure() {
    let temp = TempDir::new().unwrap();
    let test_dir = temp.path();
    create_test_repo(test_dir);

    let router = create_test_router();

    // Sync
    post(
        &router,
        "/sync",
        json!({
            "source": { "type": "local", "path": test_dir.to_str().unwrap() },
            "ignore": [],
            "no_save": false,
            "snapshot": false,
            "use_cache": false,
            "force": false
        }),
    )
    .await;

    // Graph
    let (status, json) = post(
        &router,
        "/graph",
        json!({
            "path": test_dir.to_str().unwrap(),
            "output": null,
            "force": false
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);

    let data = &json["data"];
    assert!(data["graph"]["nodes"].is_array(), "Should have nodes array");
    assert!(data["graph"]["edges"].is_array(), "Should have edges array");
    assert!(
        data["graph"]["metadata"].is_object(),
        "Should have metadata"
    );

    // Verify nodes have required fields (if any)
    let nodes = data["graph"]["nodes"].as_array().unwrap();
    if !nodes.is_empty() {
        let node = &nodes[0];
        // Note: nodes may have different field names depending on implementation
        // Just verify the array exists and has objects
        assert!(node.is_object(), "Node should be an object");
    }
}

// =============================================================================
// Load Endpoint Tests
// =============================================================================

#[tokio::test]
async fn test_load_after_sync() {
    let temp = TempDir::new().unwrap();
    let test_dir = temp.path();
    create_test_repo(test_dir);

    let router = create_test_router();

    // First sync
    post(
        &router,
        "/sync",
        json!({
            "source": { "type": "local", "path": test_dir.to_str().unwrap() },
            "ignore": [],
            "no_save": false,
            "snapshot": false,
            "use_cache": false,
            "force": false
        }),
    )
    .await;

    // Then load
    let uri = format!("/load?path={}", test_dir.to_str().unwrap());
    let (status, json) = get(&router, &uri).await;

    assert_eq!(status, StatusCode::OK, "Load should succeed: {:?}", json);

    let data = &json["data"];
    assert!(data["project"].is_object(), "Should have project");
    assert!(data["manifest"].is_object(), "Should have manifest");
}

#[tokio::test]
async fn test_load_fails_without_sync() {
    let temp = TempDir::new().unwrap();
    let test_dir = temp.path();
    create_test_repo(test_dir);

    let router = create_test_router();

    // Load without sync should fail (returns 500 with LOAD_ERROR)
    let uri = format!("/load?path={}", test_dir.to_str().unwrap());
    let (status, json) = get(&router, &uri).await;

    // Note: API returns 500 for any load error, not 404
    assert!(
        status == StatusCode::NOT_FOUND || status == StatusCode::INTERNAL_SERVER_ERROR,
        "Load should fail without sync: {:?}",
        json
    );
    assert!(json["data"]["code"].is_string(), "Should have error code");
}

// =============================================================================
// Clean Endpoint Tests
// =============================================================================

#[tokio::test]
async fn test_clean_removes_self() {
    let temp = TempDir::new().unwrap();
    let test_dir = temp.path();
    create_test_repo(test_dir);

    let router = create_test_router();

    // First sync
    post(
        &router,
        "/sync",
        json!({
            "source": { "type": "local", "path": test_dir.to_str().unwrap() },
            "ignore": [],
            "no_save": false,
            "snapshot": false,
            "use_cache": false,
            "force": false
        }),
    )
    .await;

    assert!(
        test_dir.join(".self").exists(),
        ".self should exist after sync"
    );

    // Clean
    let uri = format!("/clean?path={}", test_dir.to_str().unwrap());
    let (status, json) = delete(&router, &uri).await;

    assert_eq!(status, StatusCode::OK, "Clean should succeed: {:?}", json);

    let data = &json["data"];
    assert_eq!(
        data["cleaned"].as_bool(),
        Some(true),
        "Should report cleaned"
    );
    assert!(
        !test_dir.join(".self").exists(),
        ".self should be removed after clean"
    );
}

#[tokio::test]
async fn test_clean_on_nonexistent_self() {
    let temp = TempDir::new().unwrap();
    let test_dir = temp.path();
    create_test_repo(test_dir);

    let router = create_test_router();

    // Clean without sync
    let uri = format!("/clean?path={}", test_dir.to_str().unwrap());
    let (status, json) = delete(&router, &uri).await;

    assert_eq!(status, StatusCode::OK, "Clean should succeed: {:?}", json);

    let data = &json["data"];
    assert_eq!(
        data["cleaned"].as_bool(),
        Some(false),
        "Should report not cleaned (nothing to clean)"
    );
}

// =============================================================================
// Git Changes Endpoint Tests
// =============================================================================

#[tokio::test]
async fn test_git_changes_after_sync() {
    let temp = TempDir::new().unwrap();
    let test_dir = temp.path();
    create_test_repo(test_dir);

    let router = create_test_router();

    // First sync
    post(
        &router,
        "/sync",
        json!({
            "source": { "type": "local", "path": test_dir.to_str().unwrap() },
            "ignore": [],
            "no_save": false,
            "snapshot": false,
            "use_cache": false,
            "force": false
        }),
    )
    .await;

    // Get git changes
    let uri = format!("/git-changes?path={}", test_dir.to_str().unwrap());
    let (status, json) = get(&router, &uri).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "Git changes should succeed: {:?}",
        json
    );

    let data = &json["data"];
    // Git changes should return some structure (might be empty if no changes)
    assert!(data.is_object(), "Should return an object");
}

// =============================================================================
// Response Wrapper Tests
// =============================================================================

#[tokio::test]
async fn test_api_response_wrapper() {
    let temp = TempDir::new().unwrap();
    let test_dir = temp.path();
    create_test_repo(test_dir);

    let router = create_test_router();

    let uri = format!("/status?path={}", test_dir.to_str().unwrap());
    let (status, json) = get(&router, &uri).await;

    assert_eq!(status, StatusCode::OK);

    // All responses should be wrapped in ApiResponse with a "data" field
    assert!(
        json.get("data").is_some(),
        "Response should have 'data' field: {:?}",
        json
    );
}

#[tokio::test]
async fn test_error_response_structure() {
    let temp = TempDir::new().unwrap();
    let test_dir = temp.path();
    create_test_repo(test_dir);

    let router = create_test_router();

    // Load without sync should return an error
    let uri = format!("/load?path={}", test_dir.to_str().unwrap());
    let (status, json) = get(&router, &uri).await;

    // API returns 500 for load errors
    assert!(
        status == StatusCode::NOT_FOUND || status == StatusCode::INTERNAL_SERVER_ERROR,
        "Load should return error status"
    );

    // Error responses should have error info in data
    let data = &json["data"];
    assert!(data["code"].is_string(), "Error should have code");
    assert!(data["message"].is_string(), "Error should have message");
}

// =============================================================================
// Workflow Tests (Full API Usage)
// =============================================================================

#[tokio::test]
async fn test_full_workflow_sync_graph_clean() {
    let temp = TempDir::new().unwrap();
    let test_dir = temp.path();
    create_test_repo(test_dir);

    let router = create_test_router();

    // 1. Sync
    let (status, _json) = post(
        &router,
        "/sync",
        json!({
            "source": { "type": "local", "path": test_dir.to_str().unwrap() },
            "ignore": [],
            "no_save": false,
            "snapshot": false,
            "use_cache": false,
            "force": false
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "Sync should succeed");
    assert!(test_dir.join(".self").exists());

    // 2. Status
    let uri = format!("/status?path={}", test_dir.to_str().unwrap());
    let (status, json) = get(&router, &uri).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["data"]["store_exists"].as_bool(), Some(true));

    // 3. Graph
    let (status, _json) = post(
        &router,
        "/graph",
        json!({
            "path": test_dir.to_str().unwrap(),
            "output": null,
            "force": false
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "Graph should succeed");
    assert!(test_dir.join(".self/graph.json").exists());

    // 4. Load
    let uri = format!("/load?path={}", test_dir.to_str().unwrap());
    let (status, json) = get(&router, &uri).await;
    assert_eq!(status, StatusCode::OK, "Load should succeed");
    assert!(json["data"]["project"].is_object());

    // 5. Clean
    let uri = format!("/clean?path={}", test_dir.to_str().unwrap());
    let (status, _json) = delete(&router, &uri).await;
    assert_eq!(status, StatusCode::OK, "Clean should succeed");
    assert!(!test_dir.join(".self").exists());

    // 6. Verify clean worked - load should now fail
    let uri = format!("/load?path={}", test_dir.to_str().unwrap());
    let (status, _) = get(&router, &uri).await;
    // API returns 500 for load errors, not 404
    assert!(
        status == StatusCode::NOT_FOUND || status == StatusCode::INTERNAL_SERVER_ERROR,
        "Load should fail after clean"
    );
}

#[tokio::test]
async fn test_resync_updates_project() {
    let temp = TempDir::new().unwrap();
    let test_dir = temp.path();
    create_test_repo(test_dir);

    let router = create_test_router();

    // Initial sync
    let (status, json1) = post(
        &router,
        "/sync",
        json!({
            "source": { "type": "local", "path": test_dir.to_str().unwrap() },
            "ignore": [],
            "no_save": false,
            "snapshot": false,
            "use_cache": false,
            "force": false
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "Initial sync should succeed: {:?}",
        json1
    );

    // Verify sync returned project data
    assert!(json1["data"]["project"].is_object(), "Should have project");

    // Add a new file
    fs::write(test_dir.join("new_file.rs"), "pub fn new() {}").unwrap();

    // Resync with force
    let (status, json2) = post(
        &router,
        "/sync",
        json!({
            "source": { "type": "local", "path": test_dir.to_str().unwrap() },
            "ignore": [],
            "no_save": false,
            "snapshot": false,
            "use_cache": false,
            "force": true
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "Resync should succeed: {:?}", json2);

    // Verify resync returned project data
    assert!(
        json2["data"]["project"].is_object(),
        "Should have project after resync"
    );

    // Verify .self is updated (exists and accessible)
    assert!(
        test_dir.join(".self").exists(),
        ".self should exist after resync"
    );
}

// =============================================================================
// Backward Compatibility Tests
// =============================================================================

#[tokio::test]
async fn test_load_backward_compatible_project_json() {
    let temp = TempDir::new().unwrap();
    let test_dir = temp.path();
    create_test_repo(test_dir);

    // Create .self directory manually with old format
    fs::create_dir_all(test_dir.join(".self")).unwrap();

    // Create a project.json in the OLD format (externally tagged, not internally tagged)
    // This is the format that was used before the ops layer refactoring
    let old_format_project = serde_json::json!({
        "name": "test-workspace",
        "source": {
            "LocalPath": {
                "path": test_dir.to_str().unwrap()
            }
        },
        "repositories": [
            {
                "name": "test-repo",
                "url": test_dir.to_str().unwrap(),
                "local_path": test_dir.to_str().unwrap(),
                "sources": []
            }
        ]
    });
    fs::write(
        test_dir.join(".self/project.json"),
        serde_json::to_string_pretty(&old_format_project).unwrap(),
    )
    .unwrap();

    // Create minimal manifest (version is u32, last_sync is SystemTime serialized format)
    let manifest = serde_json::json!({
        "version": 1,
        "name": "test-workspace",
        "root": test_dir.to_str().unwrap(),
        "kind": "SingleRepo",
        "last_sync": {
            "secs_since_epoch": 1704067200,
            "nanos_since_epoch": 0
        },
        "repo_count": 1,
        "source_count": 0,
        "total_size": 0,
        "remote": null
    });
    fs::write(
        test_dir.join(".self/manifest.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let router = create_test_router();

    // Load should succeed with the old format
    let uri = format!("/load?path={}", test_dir.to_str().unwrap());
    let (status, json) = get(&router, &uri).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "Load should succeed with old project.json format: {:?}",
        json
    );
    assert!(json["data"]["project"].is_object(), "Should have project");
}

#[tokio::test]
async fn test_multi_repo_git_changes() {
    let temp = TempDir::new().unwrap();
    let test_dir = temp.path();
    create_multi_repo_workspace(test_dir);

    let router = create_test_router();

    // Sync the multi-repo workspace
    let (status, _) = post(
        &router,
        "/sync",
        json!({
            "source": { "type": "local", "path": test_dir.to_str().unwrap() },
            "ignore": [],
            "no_save": false,
            "snapshot": false,
            "use_cache": false,
            "force": false
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Get git changes - should work for multi-repo workspace
    let uri = format!("/git-changes?path={}", test_dir.to_str().unwrap());
    let (status, json) = get(&router, &uri).await;

    // Should succeed (not fail with "Failed to open repository")
    assert_eq!(
        status,
        StatusCode::OK,
        "Git changes should work for multi-repo workspace: {:?}",
        json
    );
}
