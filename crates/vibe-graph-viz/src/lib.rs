//! WASM-compatible egui visualization for SourceCodeGraph.
//!
//! This crate provides an egui-based visualization that can run:
//! - Natively (via eframe)
//! - In the browser (via WASM)
//!
//! ## Module Structure
//!
//! - `app` - Main application state and update loop
//! - `selection` - Lasso selection and neighborhood expansion
//! - `settings` - UI settings structures
//! - `ui` - Overlay rendering (lasso, indicators)
//! - `sample` - Sample graph generation

mod app;
mod sample;
mod selection;
mod settings;
mod ui;

#[cfg(feature = "automaton")]
pub mod automaton_app;

pub use app::VibeGraphApp;

#[cfg(feature = "automaton")]
pub use automaton_app::AutomatonVizApp;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

/// Start the visualization app in WASM context.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(start)]
pub fn wasm_main() {
    use wasm_bindgen::JsCast;

    // Better panic messages in the browser console
    console_error_panic_hook::set_once();

    let web_options = eframe::WebOptions::default();

    wasm_bindgen_futures::spawn_local(async {
        // Get the canvas element from the DOM
        let document = web_sys::window()
            .expect("No window")
            .document()
            .expect("No document");

        let canvas = document
            .get_element_by_id("vibe-graph-canvas")
            .expect("No canvas element with id 'vibe-graph-canvas'")
            .dyn_into::<web_sys::HtmlCanvasElement>()
            .expect("Element is not a canvas");

        eframe::WebRunner::new()
            .start(
                canvas,
                web_options,
                Box::new(|cc| Ok(Box::new(VibeGraphApp::new(cc)))),
            )
            .await
            .expect("Failed to start eframe");
    });
}

/// Load graph data from JSON (called from JavaScript).
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn load_graph_json(json: &str) -> Result<(), JsValue> {
    // This would be called from JS to load graph data
    // For now, we'll use a simpler approach where the data is embedded
    web_sys::console::log_1(&format!("Received graph JSON: {} bytes", json.len()).into());
    Ok(())
}
