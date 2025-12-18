/**
 * Vibe Graph Frontend - Entry Point
 *
 * This is a minimal TypeScript layer that:
 * 1. Fetches graph data from the REST API
 * 2. Sets it on window.VIBE_GRAPH_DATA for WASM to consume
 * 3. Initializes the WASM visualization
 * 4. Connects WebSocket for real-time updates
 */

import { fetchGraph } from "./api/client";
import type { WsServerMessage } from "./api/types";
import { connectWebSocket } from "./api/websocket";

// Get loading element
function getLoadingElement(): HTMLElement | null {
  return document.getElementById("loading");
}

function showError(message: string): void {
  const loading = getLoadingElement();
  if (loading) {
    loading.classList.add("error");
    loading.innerHTML = `<span>Error: ${message}</span>`;
  }
}

function updateLoadingText(text: string): void {
  const loading = getLoadingElement();
  if (loading) {
    const span = loading.querySelector("span");
    if (span) {
      span.textContent = text;
    }
  }
}

// Extend window type for WASM communication
declare global {
  interface Window {
    VIBE_GRAPH_DATA?: string;
    VIBE_GIT_CHANGES?: string;
  }
}

/**
 * Handle incoming WebSocket messages.
 */
function handleWsMessage(message: WsServerMessage): void {
  switch (message.type) {
    case "git_changes":
      // Set on window for WASM to pick up (polled in update loop)
      window.VIBE_GIT_CHANGES = JSON.stringify(message.data);
      console.log(
        `[WS] Git changes updated: ${message.data.changes.length} changes`
      );
      break;

    case "graph_updated":
      console.log(
        `[WS] Graph updated: ${message.node_count} nodes, ${message.edge_count} edges`
      );
      // Could trigger a graph refresh here
      break;

    case "error":
      console.error(`[WS] Server error: ${message.code} - ${message.message}`);
      break;

    case "pong":
      // Heartbeat response, ignore
      break;
  }
}

/**
 * Main initialization sequence.
 */
async function main(): Promise<void> {
  try {
    // 1. Fetch graph data from API
    updateLoadingText("Fetching graph data...");
    const graph = await fetchGraph();
    console.log(
      `[API] Loaded graph: ${graph.nodes.length} nodes, ${graph.edges.length} edges`
    );

    // 2. Set on window for WASM to pick up
    window.VIBE_GRAPH_DATA = JSON.stringify(graph);

    // 3. Signal that data is ready for WASM initialization
    updateLoadingText("Loading visualization...");
    window.dispatchEvent(new CustomEvent("vibe-graph-ready"));

    // 4. Connect WebSocket for real-time updates
    connectWebSocket(handleWsMessage, {
      onOpen: () => console.log("[Main] WebSocket connected"),
      onClose: () => console.log("[Main] WebSocket disconnected"),
    });
  } catch (error) {
    console.error("[Main] Initialization failed:", error);
    showError(error instanceof Error ? error.message : "Unknown error");
  }
}

// Start the application
main();
