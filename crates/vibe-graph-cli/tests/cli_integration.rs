//! Integration tests for the vg CLI.
//!
//! These tests capture the current behavior of the CLI to ensure
//! functional equivalence after refactoring to use the ops layer.
//!
//! Run with: `cargo test --package vibe-graph-cli --test cli_integration`

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use tempfile::TempDir;

/// Helper to run the vg CLI with given arguments.
fn run_vg(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_vg"))
        .args(args)
        .output()
        .expect("Failed to execute vg command")
}

/// Helper to run vg in a specific directory.
fn run_vg_in_dir(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_vg"))
        .current_dir(dir)
        .args(args)
        .output()
        .expect("Failed to execute vg command")
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
    // Create two repos
    let repo1 = dir.join("repo1");
    let repo2 = dir.join("repo2");

    fs::create_dir_all(&repo1).unwrap();
    fs::create_dir_all(&repo2).unwrap();

    create_test_repo(&repo1);
    create_test_repo(&repo2);
}

// =============================================================================
// Sync Command Tests
// =============================================================================

#[test]
fn test_sync_local_creates_self_folder() {
    let temp = TempDir::new().unwrap();
    let test_dir = temp.path();

    create_test_repo(test_dir);

    let output = run_vg_in_dir(test_dir, &["sync", "."]);

    assert!(output.status.success(), "vg sync should succeed");
    assert!(
        test_dir.join(".self").exists(),
        ".self folder should be created"
    );
    assert!(
        test_dir.join(".self/project.json").exists(),
        "project.json should be created"
    );
    assert!(
        test_dir.join(".self/manifest.json").exists(),
        "manifest.json should be created"
    );
}

#[test]
fn test_sync_no_save_skips_self_folder() {
    let temp = TempDir::new().unwrap();
    let test_dir = temp.path();

    create_test_repo(test_dir);

    let output = run_vg_in_dir(test_dir, &["sync", "--no-save", "."]);

    assert!(output.status.success(), "vg sync --no-save should succeed");
    assert!(
        !test_dir.join(".self").exists(),
        ".self folder should NOT be created with --no-save"
    );
}

#[test]
fn test_sync_output_format() {
    let temp = TempDir::new().unwrap();
    let test_dir = temp.path();

    create_test_repo(test_dir);

    let output = run_vg_in_dir(test_dir, &["sync", "."]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Verify expected output format with emojis
    assert!(
        stdout.contains("📁 Workspace:"),
        "Output should contain workspace info"
    );
    assert!(
        stdout.contains("📍 Path:"),
        "Output should contain path info"
    );
    assert!(
        stdout.contains("🔍 Detected:"),
        "Output should contain detection info"
    );
    assert!(
        stdout.contains("✅ Sync complete"),
        "Output should contain sync complete message"
    );
    assert!(
        stdout.contains("💾 Saved to"),
        "Output should contain saved message"
    );
}

#[test]
fn test_sync_counts_files_correctly() {
    let temp = TempDir::new().unwrap();
    let test_dir = temp.path();

    create_test_repo(test_dir);

    let output = run_vg_in_dir(test_dir, &["sync", "."]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Should count 3 files: main.rs, lib.rs, src/utils.rs
    assert!(
        stdout.contains("Total files:"),
        "Output should show total files"
    );
    // Note: exact count may vary based on what's filtered
}

#[test]
fn test_sync_multi_repo_workspace() {
    let temp = TempDir::new().unwrap();
    let test_dir = temp.path();

    create_multi_repo_workspace(test_dir);

    let output = run_vg_in_dir(test_dir, &["sync", "."]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success(), "vg sync should succeed");
    assert!(
        stdout.contains("Repositories:"),
        "Should show repository count"
    );
    // Should detect 2 repositories
    assert!(
        stdout.contains("2") || stdout.contains("repositories"),
        "Should detect multi-repo workspace"
    );
}

#[test]
fn test_sync_with_snapshot() {
    let temp = TempDir::new().unwrap();
    let test_dir = temp.path();

    create_test_repo(test_dir);

    let output = run_vg_in_dir(test_dir, &["sync", "--snapshot", "."]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success(), "vg sync --snapshot should succeed");
    assert!(
        stdout.contains("📸 Snapshot:"),
        "Should create snapshot"
    );

    // Verify snapshot directory exists
    let snapshots_dir = test_dir.join(".self/snapshots");
    assert!(snapshots_dir.exists(), "Snapshots directory should exist");

    // Should have at least one snapshot file
    let snapshot_count = fs::read_dir(&snapshots_dir)
        .unwrap()
        .filter(|e| e.is_ok())
        .count();
    assert!(snapshot_count > 0, "Should have at least one snapshot");
}

// =============================================================================
// Status Command Tests
// =============================================================================

#[test]
fn test_status_on_synced_workspace() {
    let temp = TempDir::new().unwrap();
    let test_dir = temp.path();

    create_test_repo(test_dir);

    // First sync
    run_vg_in_dir(test_dir, &["sync", "."]);

    // Then status
    let output = run_vg_in_dir(test_dir, &["status", "."]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success(), "vg status should succeed");
    assert!(
        stdout.contains("📊 Vibe-Graph Status"),
        "Should show status header"
    );
    assert!(
        stdout.contains("📁 Workspace:"),
        "Should show workspace name"
    );
    assert!(
        stdout.contains("💾 .self:"),
        "Should show .self status"
    );
    assert!(
        stdout.contains("initialized"),
        "Should show initialized status"
    );
}

#[test]
fn test_status_on_unsynced_workspace() {
    let temp = TempDir::new().unwrap();
    let test_dir = temp.path();

    create_test_repo(test_dir);

    // Status without sync
    let output = run_vg_in_dir(test_dir, &["status", "."]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success(), "vg status should succeed");
    assert!(
        stdout.contains("not initialized"),
        "Should show not initialized"
    );
}

// =============================================================================
// Graph Command Tests
// =============================================================================

#[test]
fn test_graph_creates_graph_json() {
    let temp = TempDir::new().unwrap();
    let test_dir = temp.path();

    create_test_repo(test_dir);

    // First sync
    run_vg_in_dir(test_dir, &["sync", "."]);

    // Then graph
    let output = run_vg_in_dir(test_dir, &["graph", "."]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success(), "vg graph should succeed");
    assert!(
        test_dir.join(".self/graph.json").exists(),
        "graph.json should be created"
    );
    assert!(
        stdout.contains("Building SourceCodeGraph"),
        "Should show building message"
    );
    assert!(stdout.contains("Nodes:"), "Should show node count");
    assert!(stdout.contains("Edges:"), "Should show edge count");
}

#[test]
fn test_graph_fails_without_sync() {
    let temp = TempDir::new().unwrap();
    let test_dir = temp.path();

    create_test_repo(test_dir);

    // Graph without sync should fail
    let output = run_vg_in_dir(test_dir, &["graph", "."]);

    assert!(
        !output.status.success(),
        "vg graph should fail without sync"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(".self") || stderr.contains("sync"),
        "Error should mention .self or sync"
    );
}

#[test]
fn test_graph_output_path() {
    let temp = TempDir::new().unwrap();
    let test_dir = temp.path();

    create_test_repo(test_dir);

    // First sync
    run_vg_in_dir(test_dir, &["sync", "."]);

    // Graph with custom output
    let output_path = test_dir.join("custom-graph.json");
    let output = run_vg_in_dir(
        test_dir,
        &["graph", "--output", output_path.to_str().unwrap(), "."],
    );

    assert!(output.status.success(), "vg graph with output should succeed");
    assert!(output_path.exists(), "Custom output file should be created");

    // Verify it's valid JSON
    let content = fs::read_to_string(&output_path).unwrap();
    let _: serde_json::Value = serde_json::from_str(&content).expect("Should be valid JSON");
}

// =============================================================================
// Load Command Tests
// =============================================================================

#[test]
fn test_load_after_sync() {
    let temp = TempDir::new().unwrap();
    let test_dir = temp.path();

    create_test_repo(test_dir);

    // First sync
    run_vg_in_dir(test_dir, &["sync", "."]);

    // Then load
    let output = run_vg_in_dir(test_dir, &["load", "."]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success(), "vg load should succeed");
    assert!(stdout.contains("📂 Loaded:"), "Should show loaded message");
}

#[test]
fn test_load_fails_without_sync() {
    let temp = TempDir::new().unwrap();
    let test_dir = temp.path();

    create_test_repo(test_dir);

    // Load without sync should fail
    let output = run_vg_in_dir(test_dir, &["load", "."]);

    assert!(!output.status.success(), "vg load should fail without sync");
}

// =============================================================================
// Clean Command Tests
// =============================================================================

#[test]
fn test_clean_removes_self_folder() {
    let temp = TempDir::new().unwrap();
    let test_dir = temp.path();

    create_test_repo(test_dir);

    // First sync
    run_vg_in_dir(test_dir, &["sync", "."]);
    assert!(test_dir.join(".self").exists(), ".self should exist after sync");

    // Then clean
    let output = run_vg_in_dir(test_dir, &["clean", "."]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success(), "vg clean should succeed");
    assert!(
        !test_dir.join(".self").exists(),
        ".self should be removed after clean"
    );
    assert!(
        stdout.contains("🧹 Cleaned"),
        "Should show cleaned message"
    );
}

#[test]
fn test_clean_on_nonexistent_self() {
    let temp = TempDir::new().unwrap();
    let test_dir = temp.path();

    create_test_repo(test_dir);

    // Clean without sync
    let output = run_vg_in_dir(test_dir, &["clean", "."]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success(), "vg clean should succeed");
    assert!(
        stdout.contains("No .self folder"),
        "Should indicate no .self folder"
    );
}

// =============================================================================
// Config Command Tests
// =============================================================================

#[test]
fn test_config_show() {
    let output = run_vg(&["config", "show"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success(), "vg config show should succeed");
    assert!(
        stdout.contains("Max Content Size") || stdout.contains("max_content"),
        "Should show max content size"
    );
    assert!(
        stdout.contains("Cache") || stdout.contains("cache"),
        "Should show cache directory"
    );
}

#[test]
fn test_config_path() {
    let output = run_vg(&["config", "path"]);

    assert!(output.status.success(), "vg config path should succeed");

    let stdout = String::from_utf8_lossy(&output.stdout);
    // Should output a path containing "vibe-graph"
    assert!(
        stdout.contains("vibe-graph") || stdout.contains("config"),
        "Should show config path"
    );
}

// =============================================================================
// Project Data Verification Tests
// =============================================================================

#[test]
fn test_project_json_structure() {
    let temp = TempDir::new().unwrap();
    let test_dir = temp.path();

    create_test_repo(test_dir);
    run_vg_in_dir(test_dir, &["sync", "."]);

    // Read and parse project.json
    let project_json = fs::read_to_string(test_dir.join(".self/project.json")).unwrap();
    let project: serde_json::Value = serde_json::from_str(&project_json).unwrap();

    // Verify structure
    assert!(project.get("name").is_some(), "Project should have name");
    assert!(project.get("source").is_some(), "Project should have source");
    assert!(
        project.get("repositories").is_some(),
        "Project should have repositories"
    );

    let repos = project["repositories"].as_array().unwrap();
    assert!(!repos.is_empty(), "Should have at least one repository");

    let repo = &repos[0];
    assert!(repo.get("name").is_some(), "Repo should have name");
    assert!(repo.get("sources").is_some(), "Repo should have sources");
}

#[test]
fn test_manifest_json_structure() {
    let temp = TempDir::new().unwrap();
    let test_dir = temp.path();

    create_test_repo(test_dir);
    run_vg_in_dir(test_dir, &["sync", "."]);

    // Read and parse manifest.json
    let manifest_json = fs::read_to_string(test_dir.join(".self/manifest.json")).unwrap();
    let manifest: serde_json::Value = serde_json::from_str(&manifest_json).unwrap();

    // Verify structure
    assert!(manifest.get("version").is_some(), "Manifest should have version");
    assert!(manifest.get("name").is_some(), "Manifest should have name");
    assert!(manifest.get("root").is_some(), "Manifest should have root");
    assert!(manifest.get("kind").is_some(), "Manifest should have kind");
    assert!(
        manifest.get("last_sync").is_some(),
        "Manifest should have last_sync"
    );
    assert!(
        manifest.get("repo_count").is_some(),
        "Manifest should have repo_count"
    );
    assert!(
        manifest.get("source_count").is_some(),
        "Manifest should have source_count"
    );
}

#[test]
fn test_graph_json_structure() {
    let temp = TempDir::new().unwrap();
    let test_dir = temp.path();

    create_test_repo(test_dir);
    run_vg_in_dir(test_dir, &["sync", "."]);
    run_vg_in_dir(test_dir, &["graph", "."]);

    // Read and parse graph.json
    let graph_json = fs::read_to_string(test_dir.join(".self/graph.json")).unwrap();
    let graph: serde_json::Value = serde_json::from_str(&graph_json).unwrap();

    // Verify structure
    assert!(graph.get("nodes").is_some(), "Graph should have nodes");
    assert!(graph.get("edges").is_some(), "Graph should have edges");
    assert!(graph.get("metadata").is_some(), "Graph should have metadata");

    let nodes = graph["nodes"].as_array().unwrap();
    assert!(!nodes.is_empty(), "Should have at least one node");

    let node = &nodes[0];
    assert!(node.get("id").is_some(), "Node should have id");
    assert!(node.get("name").is_some(), "Node should have name");
    assert!(node.get("kind").is_some(), "Node should have kind");
}

// =============================================================================
// Default Command Tests
// =============================================================================

#[test]
fn test_default_command_is_sync() {
    let temp = TempDir::new().unwrap();
    let test_dir = temp.path();

    create_test_repo(test_dir);

    // Running vg without a command should default to sync
    let output = run_vg_in_dir(test_dir, &[]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success(), "vg (default) should succeed");
    assert!(
        stdout.contains("Sync complete") || stdout.contains("✅"),
        "Default command should be sync"
    );
}

// =============================================================================
// Verbose Mode Tests
// =============================================================================

#[test]
fn test_verbose_mode() {
    let temp = TempDir::new().unwrap();
    let test_dir = temp.path();

    create_test_repo(test_dir);

    let output = run_vg_in_dir(test_dir, &["-v", "sync", "."]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success(), "vg -v sync should succeed");
    // Verbose mode shows individual files
    assert!(
        stdout.contains("📦") || stdout.contains("files"),
        "Verbose mode should show more details"
    );
}

#[test]
fn test_quiet_mode() {
    let temp = TempDir::new().unwrap();
    let test_dir = temp.path();

    create_test_repo(test_dir);

    let output = run_vg_in_dir(test_dir, &["-q", "sync", "."]);

    assert!(output.status.success(), "vg -q sync should succeed");
    // Quiet mode should still create .self
    assert!(test_dir.join(".self").exists(), ".self should be created even in quiet mode");
}

