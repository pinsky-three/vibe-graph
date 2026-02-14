//! File viewer windows with syntax highlighting.
//!
//! Opens egui windows that display file contents with syntax highlighting
//! for supported languages (.rs, .py, .md, .toml, .ts, .js, .json, etc.).
//! Uses `egui_extras::syntax_highlighting` backed by syntect.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use egui::{Context, ScrollArea, TextStyle};
use egui_extras::syntax_highlighting::{self, CodeTheme};

// =============================================================================
// Language detection
// =============================================================================

/// Map file extension to syntect language token.
/// Returns `None` for extensions we don't want to render (binary, etc.).
fn language_for_ext(ext: &str) -> Option<&'static str> {
    match ext {
        "rs" => Some("rs"),
        "py" | "pyi" => Some("py"),
        "md" | "markdown" => Some("md"),
        "toml" => Some("toml"),
        "ts" | "tsx" => Some("ts"),
        "js" | "jsx" | "mjs" | "cjs" => Some("js"),
        "json" => Some("json"),
        "yaml" | "yml" => Some("yaml"),
        "html" | "htm" => Some("html"),
        "css" | "scss" | "sass" => Some("css"),
        "sh" | "bash" | "zsh" => Some("sh"),
        "sql" => Some("sql"),
        "c" | "h" => Some("c"),
        "cpp" | "cxx" | "cc" | "hpp" => Some("cpp"),
        "go" => Some("go"),
        "java" => Some("java"),
        "xml" | "svg" => Some("xml"),
        "txt" | "log" | "cfg" | "ini" | "env" => Some("txt"),
        "dockerfile" => Some("dockerfile"),
        _ => None,
    }
}

/// Detect language from a file path (extension or special filenames).
fn detect_language(path: &Path) -> &'static str {
    // Check special filenames first (case-insensitive)
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

    // Then try extension
    path.extension()
        .and_then(|ext| ext.to_str())
        .and_then(|ext| language_for_ext(&ext.to_lowercase()))
        .unwrap_or("txt")
}

// =============================================================================
// Single file window state
// =============================================================================

/// State for a single open file viewer window.
#[derive(Debug, Clone)]
pub struct FileWindow {
    /// Absolute or relative path to the file.
    pub path: PathBuf,
    /// Cached file content (loaded once on open).
    pub content: String,
    /// Detected language for syntax highlighting.
    pub language: String,
    /// Whether this window is currently open (egui close button sets to false).
    pub open: bool,
    /// Unique window ID suffix (to allow the same file opened twice if needed).
    id: u64,
}

impl FileWindow {
    /// Render this file window. Returns `false` when the window was closed.
    fn show(&mut self, ctx: &Context) -> bool {
        let title = self
            .path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file");

        let window_id = format!("file_viewer_{}_{}", self.id, title);

        egui::Window::new(format!("📄 {}", title))
            .id(egui::Id::new(&window_id))
            .open(&mut self.open)
            .default_size([620.0, 480.0])
            .resizable(true)
            .collapsible(true)
            .scroll(false)
            .show(ctx, |ui| {
                // Header: full path + language badge
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(self.path.display().to_string())
                            .small()
                            .color(egui::Color32::GRAY),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(&self.language)
                                .small()
                                .strong()
                                .color(egui::Color32::from_rgb(0, 200, 150)),
                        );
                    });
                });

                ui.separator();

                // Syntax-highlighted code view
                let theme = CodeTheme::from_memory(ui.ctx(), ui.style());

                ScrollArea::both()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        // Use monospace font
                        ui.style_mut().override_text_style = Some(TextStyle::Monospace);

                        syntax_highlighting::code_view_ui(
                            ui,
                            &theme,
                            &self.content,
                            &self.language,
                        );
                    });
            });

        self.open
    }
}

// =============================================================================
// Multi-window manager
// =============================================================================

/// Manages all open file viewer windows.
#[derive(Debug, Clone, Default)]
pub struct FileViewerState {
    /// Open file windows keyed by an incrementing ID.
    windows: HashMap<u64, FileWindow>,
    /// Next window ID counter.
    next_id: u64,
    /// Optional root path for resolving relative file paths.
    root_path: Option<PathBuf>,
}

impl FileViewerState {
    /// Create a new empty file viewer state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the root path used to resolve relative file paths.
    pub fn set_root_path(&mut self, path: PathBuf) {
        self.root_path = Some(path);
    }

    /// Open a file in a new viewer window.
    ///
    /// Reads the file from disk (native only). If the file is already open,
    /// brings it to focus instead of opening a duplicate.
    pub fn open_file(&mut self, path: &Path) {
        // Check if already open — if so, just mark it open (in case user closed it)
        for window in self.windows.values_mut() {
            if window.path == path {
                window.open = true;
                return;
            }
        }

        // Resolve path: try absolute first, then relative to root
        let resolved = if path.is_absolute() && path.exists() {
            path.to_path_buf()
        } else if let Some(root) = &self.root_path {
            let joined = root.join(path);
            if joined.exists() {
                joined
            } else {
                path.to_path_buf()
            }
        } else {
            path.to_path_buf()
        };

        // Read file content
        let content = match std::fs::read_to_string(&resolved) {
            Ok(c) => c,
            Err(e) => format!("// Error reading file: {}\n// Path: {}", e, resolved.display()),
        };

        let language = detect_language(&resolved).to_string();

        let id = self.next_id;
        self.next_id += 1;

        self.windows.insert(
            id,
            FileWindow {
                path: path.to_path_buf(),
                content,
                language,
                open: true,
                id,
            },
        );
    }

    /// Render all open file viewer windows. Removes closed ones.
    pub fn show(&mut self, ctx: &Context) {
        // Collect IDs of windows to remove (closed by user)
        let mut to_remove = Vec::new();

        for (&id, window) in self.windows.iter_mut() {
            if !window.show(ctx) {
                to_remove.push(id);
            }
        }

        for id in to_remove {
            self.windows.remove(&id);
        }
    }

    /// Number of currently open windows.
    pub fn open_count(&self) -> usize {
        self.windows.values().filter(|w| w.open).count()
    }

    /// Check if a specific file is already open.
    pub fn is_open(&self, path: &Path) -> bool {
        self.windows.values().any(|w| w.path == path && w.open)
    }

    /// Close all open file viewer windows.
    pub fn close_all(&mut self) {
        self.windows.clear();
    }
}
