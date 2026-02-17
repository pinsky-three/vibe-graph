//! Managed process — spawns, monitors, and restarts the user's program.
//!
//! When `[process]` is configured in `vg.toml`, the watch loop delegates
//! to `ManagedProcess` to keep the program running alongside the automaton.
//! Captured stderr/stdout feeds back into the evolution plan.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tracing::{debug, info, warn};

use vibe_graph_automaton::{
    parse_errors, ProcessFeedback, ProcessSection, RestartPolicy, ScriptResult,
};

/// Write to stderr without panicking on pipe errors (EAGAIN, broken pipe, etc.).
/// `eprintln!` panics when stderr is temporarily unavailable — this is fatal
/// inside tokio tasks that stream child-process output at high throughput.
macro_rules! try_eprintln {
    ($($arg:tt)*) => {{
        let _ = writeln!(std::io::stderr(), $($arg)*);
    }};
}

/// Maximum stderr lines kept in the ring buffer.
const MAX_STDERR_LINES: usize = 200;

/// Env var set on child processes to prevent recursive spawning.
/// When `vg run` is itself the managed process (self-hosting), the child
/// sees this and skips spawning its own `[process]`.
pub const VG_MANAGED_ENV: &str = "VG_MANAGED";

/// A managed child process with restart and output capture.
pub struct ManagedProcess {
    config: ProcessSection,
    cwd: PathBuf,
    child: Option<Child>,
    started_at: Option<Instant>,
    crash_count: usize,
    /// Shared buffer for stderr lines captured by the background reader.
    stderr_buf: Arc<Mutex<Vec<String>>>,
    /// Last captured exit code.
    last_exit_code: Option<i32>,
}

impl ManagedProcess {
    /// Create a new managed process (not yet spawned).
    pub fn new(config: &ProcessSection, cwd: &Path) -> Self {
        Self {
            config: config.clone(),
            cwd: cwd.to_path_buf(),
            child: None,
            started_at: None,
            crash_count: 0,
            stderr_buf: Arc::new(Mutex::new(Vec::new())),
            last_exit_code: None,
        }
    }

    /// Spawn the process. If already running, this is a no-op.
    pub fn spawn(&mut self) -> anyhow::Result<()> {
        if self.child.is_some() {
            return Ok(());
        }

        info!(cmd = %self.config.cmd, cwd = %self.cwd.display(), "Spawning managed process");

        let mut cmd = Command::new("sh");
        cmd.arg("-c")
            .arg(&self.config.cmd)
            .current_dir(&self.cwd)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        // Recursion guard: tell child processes they're managed
        cmd.env(VG_MANAGED_ENV, "1");

        // Apply extra env vars
        for (key, value) in &self.config.env {
            cmd.env(key, value);
        }

        let mut child = cmd.spawn()?;

        // Spawn background tasks to stream stdout/stderr
        if let Some(stdout) = child.stdout.take() {
            tokio::spawn(async move {
                let reader = BufReader::new(stdout);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    try_eprintln!("   [process] {}", line);
                }
            });
        }

        let stderr_buf = Arc::clone(&self.stderr_buf);
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(async move {
                let reader = BufReader::new(stderr);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    try_eprintln!("   [process:err] {}", line);
                    if let Ok(mut buf) = stderr_buf.lock() {
                        buf.push(line);
                        // Ring buffer: drop oldest if over limit
                        if buf.len() > MAX_STDERR_LINES {
                            let excess = buf.len() - MAX_STDERR_LINES;
                            buf.drain(0..excess);
                        }
                    }
                }
            });
        }

        self.child = Some(child);
        self.started_at = Some(Instant::now());
        self.last_exit_code = None;
        // Clear stderr buffer on fresh spawn
        if let Ok(mut buf) = self.stderr_buf.lock() {
            buf.clear();
        }

        try_eprintln!("   ▶ Process started: {}", self.config.cmd);
        Ok(())
    }

    /// Check if the process is still running. Returns `true` if alive.
    ///
    /// If the process has exited, captures exit code and updates state.
    pub async fn check_alive(&mut self) -> bool {
        let child = match self.child.as_mut() {
            Some(c) => c,
            None => return false,
        };

        match child.try_wait() {
            Ok(Some(status)) => {
                let code = status.code().unwrap_or(-1);
                self.last_exit_code = Some(code);
                self.child = None;

                if code != 0 {
                    self.crash_count += 1;
                    try_eprintln!(
                        "   ✖ Process exited with code {} (crash #{})",
                        code, self.crash_count
                    );
                } else {
                    try_eprintln!("   ■ Process exited normally (code 0)");
                }
                false
            }
            Ok(None) => true,
            Err(e) => {
                warn!(error = %e, "Failed to check process status");
                false
            }
        }
    }

    /// Gracefully stop the process: SIGTERM, wait grace_period, then SIGKILL.
    pub async fn kill(&mut self) {
        let child = match self.child.as_mut() {
            Some(c) => c,
            None => return,
        };

        let pid = child.id();
        debug!(pid = ?pid, "Sending SIGTERM to managed process");

        // Send SIGTERM
        #[cfg(unix)]
        if let Some(pid) = pid {
            unsafe {
                libc::kill(pid as i32, libc::SIGTERM);
            }
        }

        // Wait for grace period
        let grace = std::time::Duration::from_secs(self.config.grace_period);
        let deadline = Instant::now() + grace;

        loop {
            if let Ok(Some(_)) = child.try_wait() {
                self.child = None;
                return;
            }
            if Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }

        // Force kill
        debug!(pid = ?pid, "Grace period expired, sending SIGKILL");
        let _ = child.kill().await;
        self.child = None;
    }

    /// Restart the process: kill existing, then spawn fresh.
    pub async fn restart(&mut self) -> anyhow::Result<()> {
        if self.child.is_some() {
            try_eprintln!("   ↻ Restarting process...");
            self.kill().await;
        }
        self.spawn()
    }

    /// Handle a code change: restart if the restart policy permits it.
    pub async fn on_code_change(&mut self) -> anyhow::Result<()> {
        match self.config.restart {
            RestartPolicy::OnChange | RestartPolicy::Always => {
                self.crash_count = 0; // Reset on intentional restart
                self.restart().await
            }
            RestartPolicy::OnCrash | RestartPolicy::Never => Ok(()),
        }
    }

    /// Handle a process crash: respawn if the restart policy permits it.
    pub async fn on_crash(&mut self) -> anyhow::Result<()> {
        match self.config.restart {
            RestartPolicy::OnCrash | RestartPolicy::Always => {
                try_eprintln!("   ↻ Auto-restarting crashed process...");
                self.spawn()
            }
            RestartPolicy::OnChange | RestartPolicy::Never => Ok(()),
        }
    }

    /// Build a `ProcessFeedback` from the current state.
    pub fn to_feedback(&self) -> ProcessFeedback {
        let stderr_lines = self
            .stderr_buf
            .lock()
            .map(|buf| buf.clone())
            .unwrap_or_default();

        // Parse errors from accumulated stderr
        let combined = stderr_lines.join("\n");
        let fake_result = ScriptResult {
            name: "process".to_string(),
            cmd: self.config.cmd.clone(),
            exit_code: self.last_exit_code.unwrap_or(0),
            stdout: String::new(),
            stderr: combined,
            duration: self
                .started_at
                .map(|s| s.elapsed())
                .unwrap_or_default(),
        };
        let errors = parse_errors(&fake_result);

        ProcessFeedback {
            exit_code: self.last_exit_code,
            stderr_lines,
            errors,
            crash_count: self.crash_count,
            uptime: self
                .started_at
                .map(|s| s.elapsed())
                .unwrap_or_default(),
        }
    }

    /// Whether the process is currently running.
    #[allow(dead_code)]
    pub fn is_running(&self) -> bool {
        self.child.is_some()
    }

    /// Get the restart policy.
    #[allow(dead_code)]
    pub fn restart_policy(&self) -> RestartPolicy {
        self.config.restart
    }

    /// Get crash count.
    #[allow(dead_code)]
    pub fn crash_count(&self) -> usize {
        self.crash_count
    }
}

impl Drop for ManagedProcess {
    fn drop(&mut self) {
        // Best-effort synchronous kill on drop
        if let Some(ref mut child) = self.child {
            #[cfg(unix)]
            if let Some(pid) = child.id() {
                unsafe {
                    libc::kill(pid as i32, libc::SIGTERM);
                }
            }
            // Fallback: start_kill doesn't wait but signals the child
            let _ = child.start_kill();
        }
    }
}
