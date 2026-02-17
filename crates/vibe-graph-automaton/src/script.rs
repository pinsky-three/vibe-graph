//! Script execution and output parsing.
//!
//! Runs user-defined scripts (build, test, lint), captures their output,
//! and parses stderr/stdout for file:line error patterns. The resulting
//! `ScriptFeedback` feeds into the evolution plan as a perturbation signal.

use std::path::Path;
use std::time::{Duration, Instant};

use regex::Regex;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use crate::project_config::ProjectConfig;

// =============================================================================
// Data types
// =============================================================================

/// Result of running a single script.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptResult {
    /// Script name (e.g. "test", "lint").
    pub name: String,
    /// Command that was executed.
    pub cmd: String,
    /// Process exit code (0 = success).
    pub exit_code: i32,
    /// Captured stdout.
    pub stdout: String,
    /// Captured stderr.
    pub stderr: String,
    /// Wall-clock duration.
    pub duration: Duration,
}

impl ScriptResult {
    /// Whether the script succeeded (exit code 0).
    pub fn success(&self) -> bool {
        self.exit_code == 0
    }
}

/// A single error extracted from script output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptError {
    /// File path (as reported by the compiler/tool).
    pub file: String,
    /// Line number (0 if unknown).
    pub line: u32,
    /// Error/warning message.
    pub message: String,
    /// Which script produced this error.
    pub script: String,
    /// Severity level.
    pub severity: Severity,
}

/// Error severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Severity {
    Error,
    Warning,
}

/// Aggregated feedback from running all watch scripts.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScriptFeedback {
    /// Individual script results.
    pub results: Vec<ScriptResult>,
    /// Parsed errors from all scripts.
    pub errors: Vec<ScriptError>,
    /// How many scripts passed.
    pub passed: usize,
    /// How many scripts failed.
    pub failed: usize,
}

impl ScriptFeedback {
    /// Check if all scripts passed.
    pub fn all_passed(&self) -> bool {
        self.failed == 0
    }

    /// Get unique file paths that have errors.
    pub fn errored_files(&self) -> Vec<&str> {
        let mut files: Vec<&str> = self.errors.iter().map(|e| e.file.as_str()).collect();
        files.sort();
        files.dedup();
        files
    }

    /// Check if a given file path has errors (substring match).
    pub fn has_errors_for(&self, path: &str) -> bool {
        let lower = path.to_lowercase();
        self.errors
            .iter()
            .any(|e| lower.contains(&e.file.to_lowercase()) || e.file.to_lowercase().contains(&lower))
    }

    /// Get the first error message for a file path.
    pub fn first_error_for(&self, path: &str) -> Option<&str> {
        let lower = path.to_lowercase();
        self.errors
            .iter()
            .find(|e| lower.contains(&e.file.to_lowercase()) || e.file.to_lowercase().contains(&lower))
            .map(|e| e.message.as_str())
    }

    /// Format a one-line summary of all script results.
    pub fn summary_line(&self) -> String {
        self.results
            .iter()
            .map(|r| {
                let status = if r.success() { "OK" } else { "FAIL" };
                let errors: Vec<_> = self
                    .errors
                    .iter()
                    .filter(|e| e.script == r.name)
                    .collect();
                if errors.is_empty() {
                    format!("{}: {} ({:.1}s)", r.name, status, r.duration.as_secs_f64())
                } else {
                    format!(
                        "{}: {} ({} errors, {:.1}s)",
                        r.name,
                        status,
                        errors.len(),
                        r.duration.as_secs_f64()
                    )
                }
            })
            .collect::<Vec<_>>()
            .join(" | ")
    }
}

// =============================================================================
// Process feedback (from the managed long-running process)
// =============================================================================

/// Feedback captured from the managed process (crashes, stderr lines).
///
/// Unlike `ScriptFeedback` (batch scripts), this accumulates output over time
/// from a long-running process. Each crash or restart resets the buffer.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProcessFeedback {
    /// Last exit code (None if still running).
    pub exit_code: Option<i32>,
    /// Recent stderr lines (ring buffer, capped).
    pub stderr_lines: Vec<String>,
    /// Parsed errors from stderr.
    pub errors: Vec<ScriptError>,
    /// How many times the process has crashed since last code change.
    pub crash_count: usize,
    /// How long the process has been running (current instance).
    pub uptime: Duration,
}

impl ProcessFeedback {
    /// Check if the process crashed (has an exit code and it's non-zero).
    pub fn crashed(&self) -> bool {
        self.exit_code.map(|c| c != 0).unwrap_or(false)
    }

    /// Merge this process feedback into a `ScriptFeedback` so it can be
    /// passed uniformly to the evolution plan.
    pub fn merge_into(&self, feedback: &mut ScriptFeedback) {
        feedback.errors.extend(self.errors.clone());
        if self.crashed() {
            feedback.failed += 1;
        }
    }
}

// =============================================================================
// Execution
// =============================================================================

/// Run a single script command, capturing output.
pub fn run_script(name: &str, cmd: &str, cwd: &Path) -> ScriptResult {
    info!(script = name, cmd = cmd, cwd = %cwd.display(), "Running script");
    let start = Instant::now();

    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .current_dir(cwd)
        .output();

    let duration = start.elapsed();

    match output {
        Ok(output) => {
            let exit_code = output.status.code().unwrap_or(-1);
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();

            debug!(
                script = name,
                exit_code = exit_code,
                duration_ms = duration.as_millis() as u64,
                "Script completed"
            );

            ScriptResult {
                name: name.to_string(),
                cmd: cmd.to_string(),
                exit_code,
                stdout,
                stderr,
                duration,
            }
        }
        Err(e) => {
            warn!(script = name, error = %e, "Failed to execute script");
            ScriptResult {
                name: name.to_string(),
                cmd: cmd.to_string(),
                exit_code: -1,
                stdout: String::new(),
                stderr: format!("Failed to execute: {}", e),
                duration,
            }
        }
    }
}

/// Run all watch scripts defined in the config, sequentially.
pub fn run_watch_scripts(config: &ProjectConfig, cwd: &Path) -> ScriptFeedback {
    let watch = config.watch_scripts();
    if watch.is_empty() {
        return ScriptFeedback::default();
    }

    let mut feedback = ScriptFeedback::default();

    for (name, cmd) in &watch {
        let result = run_script(name, cmd, cwd);
        let errors = parse_errors(&result);

        if result.success() {
            feedback.passed += 1;
        } else {
            feedback.failed += 1;
        }

        feedback.errors.extend(errors);
        feedback.results.push(result);
    }

    info!(
        passed = feedback.passed,
        failed = feedback.failed,
        errors = feedback.errors.len(),
        "Watch scripts completed"
    );

    feedback
}

// =============================================================================
// Error parsing
// =============================================================================

/// Parse script output for file:line error patterns.
///
/// Uses regex patterns for common compiler/tool output formats.
/// Best-effort: unknown formats are silently skipped.
pub fn parse_errors(result: &ScriptResult) -> Vec<ScriptError> {
    let mut errors = Vec::new();
    let combined = format!("{}\n{}", result.stderr, result.stdout);

    // Rust: error[E0xxx]: message\n  --> file:line:col
    parse_rust_errors(&combined, &result.name, &mut errors);

    // GCC/Clang/ESLint: file:line:col: error|warning: message
    parse_generic_errors(&combined, &result.name, &mut errors);

    // Python: File "path", line N
    parse_python_errors(&combined, &result.name, &mut errors);

    // Go: file.go:line:col: message
    parse_go_errors(&combined, &result.name, &mut errors);

    // TypeScript: file(line,col): error TSxxxx: message
    parse_typescript_errors(&combined, &result.name, &mut errors);

    // Deduplicate by (file, line, message)
    errors.sort_by(|a, b| (&a.file, a.line, &a.message).cmp(&(&b.file, b.line, &b.message)));
    errors.dedup_by(|a, b| a.file == b.file && a.line == b.line && a.message == b.message);

    errors
}

fn parse_rust_errors(output: &str, script: &str, errors: &mut Vec<ScriptError>) {
    // Match: error[E0xxx]: message (possibly multi-line)
    //   --> file:line:col
    let re = Regex::new(r"(error|warning)(?:\[E\d+\])?: (.+)\n\s*--> (.+):(\d+):\d+").unwrap();
    for cap in re.captures_iter(output) {
        let severity = if &cap[1] == "warning" {
            Severity::Warning
        } else {
            Severity::Error
        };
        errors.push(ScriptError {
            file: cap[3].to_string(),
            line: cap[4].parse().unwrap_or(0),
            message: cap[2].trim().to_string(),
            script: script.to_string(),
            severity,
        });
    }
}

fn parse_generic_errors(output: &str, script: &str, errors: &mut Vec<ScriptError>) {
    // Match: file:line:col: error|warning: message
    let re = Regex::new(r"([^\s:]+):(\d+):\d+: (error|warning): (.+)").unwrap();
    for cap in re.captures_iter(output) {
        let severity = if &cap[3] == "warning" {
            Severity::Warning
        } else {
            Severity::Error
        };
        errors.push(ScriptError {
            file: cap[1].to_string(),
            line: cap[2].parse().unwrap_or(0),
            message: cap[4].trim().to_string(),
            script: script.to_string(),
            severity,
        });
    }
}

fn parse_python_errors(output: &str, script: &str, errors: &mut Vec<ScriptError>) {
    // Match: File "path", line N
    let re = Regex::new(r#"File "(.+)", line (\d+)"#).unwrap();
    for cap in re.captures_iter(output) {
        errors.push(ScriptError {
            file: cap[1].to_string(),
            line: cap[2].parse().unwrap_or(0),
            message: "Python error".to_string(),
            script: script.to_string(),
            severity: Severity::Error,
        });
    }
}

fn parse_go_errors(output: &str, script: &str, errors: &mut Vec<ScriptError>) {
    // Match: file.go:line:col: message
    let re = Regex::new(r"([^\s]+\.go):(\d+):\d+: (.+)").unwrap();
    for cap in re.captures_iter(output) {
        errors.push(ScriptError {
            file: cap[1].to_string(),
            line: cap[2].parse().unwrap_or(0),
            message: cap[3].trim().to_string(),
            script: script.to_string(),
            severity: Severity::Error,
        });
    }
}

fn parse_typescript_errors(output: &str, script: &str, errors: &mut Vec<ScriptError>) {
    // Match: file(line,col): error TSxxxx: message
    let re = Regex::new(r"(.+)\((\d+),\d+\): error TS\d+: (.+)").unwrap();
    for cap in re.captures_iter(output) {
        errors.push(ScriptError {
            file: cap[1].to_string(),
            line: cap[2].parse().unwrap_or(0),
            message: cap[3].trim().to_string(),
            script: script.to_string(),
            severity: Severity::Error,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_rust_error() {
        let output = r#"error[E0308]: mismatched types
 --> src/main.rs:42:5
  |
  = note: expected type `i32`
"#;
        let result = ScriptResult {
            name: "check".into(),
            cmd: "cargo check".into(),
            exit_code: 1,
            stdout: String::new(),
            stderr: output.into(),
            duration: Duration::from_secs(1),
        };
        let errors = parse_errors(&result);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].file, "src/main.rs");
        assert_eq!(errors[0].line, 42);
        assert_eq!(errors[0].message, "mismatched types");
        assert_eq!(errors[0].severity, Severity::Error);
        assert_eq!(errors[0].script, "check");
    }

    #[test]
    fn test_parse_rust_warning() {
        let output = r#"warning: unused variable: `x`
 --> src/lib.rs:10:9
  |
"#;
        let result = ScriptResult {
            name: "lint".into(),
            cmd: "cargo clippy".into(),
            exit_code: 0,
            stdout: String::new(),
            stderr: output.into(),
            duration: Duration::from_secs(1),
        };
        let errors = parse_errors(&result);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].severity, Severity::Warning);
        assert_eq!(errors[0].file, "src/lib.rs");
    }

    #[test]
    fn test_parse_generic_gcc_error() {
        let output = "src/foo.c:23:5: error: expected ';' after expression\n";
        let result = ScriptResult {
            name: "build".into(),
            cmd: "gcc".into(),
            exit_code: 1,
            stdout: String::new(),
            stderr: output.into(),
            duration: Duration::from_secs(1),
        };
        let errors = parse_errors(&result);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].file, "src/foo.c");
        assert_eq!(errors[0].line, 23);
        assert!(errors[0].message.contains("expected ';'"));
    }

    #[test]
    fn test_parse_python_error() {
        let output = r#"Traceback (most recent call last):
  File "tests/test_api.py", line 15, in test_foo
    assert False
AssertionError
"#;
        let result = ScriptResult {
            name: "test".into(),
            cmd: "pytest".into(),
            exit_code: 1,
            stdout: String::new(),
            stderr: output.into(),
            duration: Duration::from_secs(1),
        };
        let errors = parse_errors(&result);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].file, "tests/test_api.py");
        assert_eq!(errors[0].line, 15);
    }

    #[test]
    fn test_parse_go_error() {
        let output = "./main.go:12:5: undefined: foo\n";
        let result = ScriptResult {
            name: "build".into(),
            cmd: "go build".into(),
            exit_code: 1,
            stdout: output.into(),
            stderr: String::new(),
            duration: Duration::from_secs(1),
        };
        let errors = parse_errors(&result);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].file, "./main.go");
        assert_eq!(errors[0].line, 12);
        assert!(errors[0].message.contains("undefined: foo"));
    }

    #[test]
    fn test_parse_typescript_error() {
        let output = "src/app.ts(5,10): error TS2304: Cannot find name 'foo'.\n";
        let result = ScriptResult {
            name: "check".into(),
            cmd: "tsc".into(),
            exit_code: 1,
            stdout: output.into(),
            stderr: String::new(),
            duration: Duration::from_secs(1),
        };
        let errors = parse_errors(&result);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].file, "src/app.ts");
        assert_eq!(errors[0].line, 5);
        assert!(errors[0].message.contains("Cannot find name"));
    }

    #[test]
    fn test_parse_multiple_errors() {
        let output = r#"error[E0308]: mismatched types
 --> src/main.rs:10:5
error[E0425]: cannot find value `x`
 --> src/lib.rs:20:9
"#;
        let result = ScriptResult {
            name: "check".into(),
            cmd: "cargo check".into(),
            exit_code: 1,
            stdout: String::new(),
            stderr: output.into(),
            duration: Duration::from_secs(1),
        };
        let errors = parse_errors(&result);
        assert_eq!(errors.len(), 2);
    }

    #[test]
    fn test_parse_clean_output_no_errors() {
        let result = ScriptResult {
            name: "test".into(),
            cmd: "cargo test".into(),
            exit_code: 0,
            stdout: "test result: ok. 42 passed\n".into(),
            stderr: String::new(),
            duration: Duration::from_secs(1),
        };
        let errors = parse_errors(&result);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_script_feedback_summary_line() {
        let feedback = ScriptFeedback {
            results: vec![
                ScriptResult {
                    name: "check".into(),
                    cmd: "cargo check".into(),
                    exit_code: 0,
                    stdout: String::new(),
                    stderr: String::new(),
                    duration: Duration::from_millis(300),
                },
                ScriptResult {
                    name: "test".into(),
                    cmd: "cargo test".into(),
                    exit_code: 1,
                    stdout: String::new(),
                    stderr: String::new(),
                    duration: Duration::from_millis(1200),
                },
            ],
            errors: vec![ScriptError {
                file: "src/main.rs".into(),
                line: 10,
                message: "test failed".into(),
                script: "test".into(),
                severity: Severity::Error,
            }],
            passed: 1,
            failed: 1,
        };
        let summary = feedback.summary_line();
        assert!(summary.contains("check: OK"));
        assert!(summary.contains("test: FAIL (1 errors"));
    }

    #[test]
    fn test_script_feedback_errored_files() {
        let feedback = ScriptFeedback {
            errors: vec![
                ScriptError {
                    file: "src/a.rs".into(),
                    line: 1,
                    message: "err".into(),
                    script: "check".into(),
                    severity: Severity::Error,
                },
                ScriptError {
                    file: "src/b.rs".into(),
                    line: 2,
                    message: "err".into(),
                    script: "check".into(),
                    severity: Severity::Error,
                },
                ScriptError {
                    file: "src/a.rs".into(),
                    line: 5,
                    message: "err2".into(),
                    script: "check".into(),
                    severity: Severity::Error,
                },
            ],
            ..Default::default()
        };
        let files = feedback.errored_files();
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn test_run_script_echo() {
        let dir = tempfile::TempDir::new().unwrap();
        let result = run_script("test", "echo hello", dir.path());
        assert!(result.success());
        assert!(result.stdout.contains("hello"));
    }

    #[test]
    fn test_run_script_failure() {
        let dir = tempfile::TempDir::new().unwrap();
        let result = run_script("test", "false", dir.path());
        assert!(!result.success());
    }
}
