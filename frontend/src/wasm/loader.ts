/**
 * WASM loader for vibe-graph-viz.
 *
 * This module handles loading and initializing the egui WASM visualization.
 * It expects the graph data to be set on window.VIBE_GRAPH_DATA before calling init.
 */

/**
 * Initialize the WASM visualization.
 *
 * The graph data should be set on window.VIBE_GRAPH_DATA before calling this.
 *
 * Note: We load the JS glue code via a script tag because Vite doesn't allow
 * dynamic imports from /public directory.
 */
export async function initWasm(): Promise<void> {
  try {
    // Load the WASM glue code via script tag (works with /public files)
    await loadScript("/wasm/vibe_graph_viz.js");

    // The script exposes wasm_bindgen on window
    const wasmBindgen = (window as unknown as { wasm_bindgen: WasmBindgen })
      .wasm_bindgen;

    if (!wasmBindgen) {
      throw new Error("wasm_bindgen not found after loading script");
    }

    // Initialize the WASM module with the binary path
    await wasmBindgen("/wasm/vibe_graph_viz_bg.wasm");

    console.log("[WASM] Visualization initialized");
  } catch (error) {
    console.warn(
      "[WASM] Failed to load visualization. Build WASM with:",
      "\n  cd crates/vibe-graph-viz && wasm-pack build --target web --out-dir ../../frontend/public/wasm"
    );
    throw error;
  }
}

// Type for wasm-bindgen default export
interface WasmBindgen {
  (path: string): Promise<void>;
}

/**
 * Load a script dynamically and wait for it to execute.
 */
function loadScript(src: string): Promise<void> {
  return new Promise((resolve, reject) => {
    // Check if already loaded
    if (document.querySelector(`script[src="${src}"]`)) {
      resolve();
      return;
    }

    const script = document.createElement("script");
    script.src = src;
    script.type = "module";
    script.onload = () => resolve();
    script.onerror = () => reject(new Error(`Failed to load script: ${src}`));
    document.head.appendChild(script);
  });
}

/**
 * Check if WASM assets are available.
 */
export async function isWasmAvailable(): Promise<boolean> {
  try {
    const res = await fetch("/wasm/vibe_graph_viz_bg.wasm", { method: "HEAD" });
    return res.ok;
  } catch {
    return false;
  }
}
