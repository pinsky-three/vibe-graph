//! File viewer windows with syntax highlighting.
//!
//! Opens egui windows displaying file contents with syntax highlighting.
//! Uses `egui_extras::syntax_highlighting` backed by syntect.
//!
//! ## Platform support
//!
//! - **Native**: reads files from disk via `std::fs::read_to_string` (synchronous).
//! - **WASM**: fetches file content from `GET /api/file?path=...` (async via gloo-net).
//!
//! Both paths produce the same `FileContentResponse` which is then rendered
//! identically regardless of platform.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use egui::{Context, ScrollArea};
use egui_extras::syntax_highlighting::{self, CodeTheme};

#[cfg(target_arch = "wasm32")]
use crate::api::FileContentResponse;

// =============================================================================
// Loading state
// =============================================================================

/// Content loading state for a file window.
#[derive(Debug, Clone)]
#[allow(dead_code)] // `Loading` variant used only in WASM target
enum ContentState {
    /// Content is being fetched (WASM async).
    Loading,
    /// Content loaded successfully.
    Loaded {
        content: String,
        language: String,
        total_lines: usize,
        size_bytes: u64,
        /// Resolved path from the server/filesystem.
        resolved_path: String,
    },
    /// Content failed to load.
    Error(String),
}

// =============================================================================
// Single file window state
// =============================================================================

/// State for a single open file viewer window.
#[derive(Debug, Clone)]
pub struct FileWindow {
    /// Original requested path (as stored in the graph node).
    pub path: PathBuf,
    /// Content loading state.
    state: ContentState,
    /// Whether this window is currently open (egui close button sets to false).
    pub open: bool,
    /// Unique window ID.
    id: u64,
    /// Font size for the code view (user-adjustable).
    font_size: f32,
}

impl FileWindow {
    /// Render this file window. Returns `false` when the window was closed.
    fn show(&mut self, ctx: &Context) -> bool {
        let filename = self
            .path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file")
            .to_string();

        let window_id = format!("file_viewer_{}_{}", self.id, filename);

        // Snapshot state data to avoid borrow conflicts with `&mut self.open`
        let state_snapshot = self.state.clone();
        let path_display = self.path.display().to_string();
        let mut font_size = self.font_size;

        let mut open = self.open;
        egui::Window::new(format!("📄 {}", filename))
            .id(egui::Id::new(&window_id))
            .open(&mut open)
            .default_size([640.0, 480.0])
            .resizable(true)
            .collapsible(true)
            .scroll(false)
            .show(ctx, |ui| {
                match &state_snapshot {
                    ContentState::Loading => {
                        ui.vertical_centered(|ui| {
                            ui.add_space(40.0);
                            ui.spinner();
                            ui.label("Loading file...");
                        });
                    }
                    ContentState::Error(err) => {
                        ui.vertical_centered(|ui| {
                            ui.add_space(20.0);
                            ui.label(
                                egui::RichText::new("⚠ Error")
                                    .color(egui::Color32::from_rgb(255, 80, 80))
                                    .strong(),
                            );
                            ui.label(err.as_str());
                            ui.add_space(8.0);
                            ui.label(
                                egui::RichText::new(&path_display)
                                    .small()
                                    .color(egui::Color32::GRAY),
                            );
                        });
                    }
                    ContentState::Loaded {
                        content,
                        language,
                        total_lines,
                        size_bytes,
                        resolved_path,
                    } => {
                        render_loaded_content(
                            ui,
                            &mut font_size,
                            content,
                            language,
                            *total_lines,
                            *size_bytes,
                            resolved_path,
                        );
                    }
                }
            });

        // Write back mutable state
        self.open = open;
        self.font_size = font_size;
        self.open
    }

}

/// Render loaded file content (free function to avoid borrow conflicts).
fn render_loaded_content(
    ui: &mut egui::Ui,
    font_size: &mut f32,
    content: &str,
    language: &str,
    total_lines: usize,
    size_bytes: u64,
    resolved_path: &str,
) {
    // Compact header bar
    ui.horizontal(|ui| {
        // Truncated path (show last ~60 chars)
        let display_path = if resolved_path.len() > 60 {
            format!("...{}", &resolved_path[resolved_path.len() - 57..])
        } else {
            resolved_path.to_string()
        };
        ui.label(
            egui::RichText::new(display_path)
                .small()
                .color(egui::Color32::GRAY),
        );

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // Font size control
            let slider = egui::Slider::new(font_size, 7.0..=18.0)
                .step_by(0.5)
                .show_value(false);
            ui.add(slider).on_hover_text(format!("{:.0}px", *font_size));

            ui.label(
                egui::RichText::new("Aa")
                    .small()
                    .color(egui::Color32::GRAY),
            );

            ui.separator();

            // File stats
            let size_label = if size_bytes > 1024 {
                format!("{:.1}KB", size_bytes as f64 / 1024.0)
            } else {
                format!("{}B", size_bytes)
            };
            ui.label(
                egui::RichText::new(format!("{}L · {}", total_lines, size_label))
                    .small()
                    .color(egui::Color32::from_gray(100)),
            );

            ui.separator();

            // Language badge
            ui.label(
                egui::RichText::new(language)
                    .small()
                    .strong()
                    .color(egui::Color32::from_rgb(0, 200, 150)),
            );
        });
    });

    ui.separator();

    // Syntax-highlighted code with line numbers
    let theme = CodeTheme::dark(*font_size);
    let line_height = *font_size * 1.35;
    let gutter_width = gutter_width_for_lines(total_lines, *font_size);

    ScrollArea::both()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let lines: Vec<&str> = content.lines().collect();

            for (i, line) in lines.iter().enumerate() {
                let line_num = i + 1;

                ui.horizontal(|ui| {
                    // Line number gutter
                    ui.allocate_ui_with_layout(
                        egui::vec2(gutter_width, line_height),
                        egui::Layout::right_to_left(egui::Align::Min),
                        |ui| {
                            ui.label(
                                egui::RichText::new(format!("{}", line_num))
                                    .font(egui::FontId::monospace(*font_size))
                                    .color(egui::Color32::from_gray(70)),
                            );
                        },
                    );

                    // Thin separator between gutter and code
                    ui.add_space(4.0);

                    // Highlighted code line
                    let layout_job = syntax_highlighting::highlight(
                        ui.ctx(),
                        ui.style(),
                        &theme,
                        line,
                        language,
                    );
                    ui.add(egui::Label::new(layout_job).selectable(true));
                });
            }
        });
}

/// Calculate gutter width based on the number of lines.
fn gutter_width_for_lines(total_lines: usize, font_size: f32) -> f32 {
    let digits = if total_lines == 0 {
        1
    } else {
        (total_lines as f64).log10().floor() as usize + 1
    };
    // Approximate char width for monospace at the given size
    let char_width = font_size * 0.6;
    (digits as f32 * char_width) + 8.0 // +8 for right padding
}

// =============================================================================
// WASM async channel for file content results
// =============================================================================

#[cfg(target_arch = "wasm32")]
type FileResultChannel =
    std::rc::Rc<std::cell::RefCell<Option<(u64, Result<FileContentResponse, String>)>>>;

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
    /// Optional root path for resolving relative file paths (native only).
    root_path: Option<PathBuf>,
    /// Async result channel for WASM file fetches.
    #[cfg(target_arch = "wasm32")]
    result_channel: FileResultChannel,
}

impl FileViewerState {
    /// Create a new empty file viewer state.
    pub fn new() -> Self {
        Self {
            windows: HashMap::new(),
            next_id: 0,
            root_path: None,
            #[cfg(target_arch = "wasm32")]
            result_channel: std::rc::Rc::new(std::cell::RefCell::new(None)),
        }
    }

    /// Set the root path used to resolve relative file paths (native).
    pub fn set_root_path(&mut self, path: PathBuf) {
        self.root_path = Some(path);
    }

    /// Open a file in a new viewer window.
    ///
    /// - **Native**: reads synchronously from the filesystem.
    /// - **WASM**: initiates an async fetch to `/api/file?path=...`.
    pub fn open_file(&mut self, path: &Path) {
        // Check if already open — just mark it open
        for window in self.windows.values_mut() {
            if window.path == path {
                window.open = true;
                return;
            }
        }

        let id = self.next_id;
        self.next_id += 1;

        // Platform-specific content loading
        let initial_state = self.load_content(id, path);

        self.windows.insert(
            id,
            FileWindow {
                path: path.to_path_buf(),
                state: initial_state,
                open: true,
                id,
                font_size: 11.0,
            },
        );
    }

    /// Load file content — platform dispatch.
    fn load_content(&self, _id: u64, path: &Path) -> ContentState {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.load_content_native(path)
        }

        #[cfg(target_arch = "wasm32")]
        {
            self.load_content_wasm(_id, path);
            ContentState::Loading
        }
    }

    /// Native: synchronous filesystem read.
    #[cfg(not(target_arch = "wasm32"))]
    fn load_content_native(&self, path: &Path) -> ContentState {
        match crate::api::fetch_file_content_native(path, self.root_path.as_deref()) {
            Ok(resp) => ContentState::Loaded {
                content: resp.content,
                language: resp.language,
                total_lines: resp.total_lines,
                size_bytes: resp.size_bytes,
                resolved_path: resp.path,
            },
            Err(e) => ContentState::Error(e),
        }
    }

    /// WASM: async HTTP fetch.
    #[cfg(target_arch = "wasm32")]
    fn load_content_wasm(&self, id: u64, path: &Path) {
        let path_str = path.display().to_string();
        let channel = self.result_channel.clone();

        wasm_bindgen_futures::spawn_local(async move {
            let result = crate::api::fetch_file_content(&path_str).await;
            *channel.borrow_mut() = Some((id, result));
        });
    }

    /// Poll for async results (WASM). Should be called each frame.
    fn poll_results(&mut self) {
        #[cfg(target_arch = "wasm32")]
        {
            let result = self.result_channel.borrow_mut().take();
            if let Some((id, res)) = result {
                if let Some(window) = self.windows.get_mut(&id) {
                    window.state = match res {
                        Ok(resp) => ContentState::Loaded {
                            content: resp.content,
                            language: resp.language,
                            total_lines: resp.total_lines,
                            size_bytes: resp.size_bytes,
                            resolved_path: resp.path,
                        },
                        Err(e) => ContentState::Error(e),
                    };
                }
            }
        }
    }

    /// Render all open file viewer windows. Removes closed ones.
    pub fn show(&mut self, ctx: &Context) {
        // Poll for async results first
        self.poll_results();

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
