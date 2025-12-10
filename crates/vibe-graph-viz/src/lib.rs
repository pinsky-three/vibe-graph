//! WASM-compatible egui visualization for SourceCodeGraph.
//!
//! This crate provides an egui-based visualization that can run:
//! - Natively (via eframe)
//! - In the browser (via WASM)

mod app;
mod settings;

pub use app::VibeGraphApp;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

/// Start the visualization app in WASM context.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(start)]
pub fn wasm_main() {
    // Better panic messages in the browser console
    console_error_panic_hook::set_once();

    let web_options = eframe::WebOptions::default();

    wasm_bindgen_futures::spawn_local(async {
        eframe::WebRunner::new()
            .start(
                "vibe-graph-canvas",
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
