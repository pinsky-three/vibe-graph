/**
 * Top Bar Component - Operations Control Panel
 *
 * Provides UI controls for all vibe-graph operations:
 * - Sync: Scan and index codebase
 * - Graph: Build source code graph
 * - Status: Show workspace info
 * - Load: Load from .self store
 * - Clean: Remove .self folder
 * - Git Changes: Show modified files
 */

import * as ops from "../api/ops";

type OperationState = "idle" | "loading" | "success" | "error";

interface TopBarState {
  currentPath: string;
  operationState: OperationState;
  lastMessage: string;
  statusInfo: ops.StatusResponse | null;
}

const state: TopBarState = {
  currentPath: ".",
  operationState: "idle",
  lastMessage: "",
  statusInfo: null,
};

// =============================================================================
// DOM Elements
// =============================================================================

let container: HTMLElement | null = null;
let pathInput: HTMLInputElement | null = null;
let statusDisplay: HTMLElement | null = null;
let messageDisplay: HTMLElement | null = null;

// =============================================================================
// Helpers
// =============================================================================

function setMessage(msg: string, isError = false): void {
  state.lastMessage = msg;
  if (messageDisplay) {
    messageDisplay.textContent = msg;
    messageDisplay.className = `topbar-message ${isError ? "error" : "success"}`;
    // Auto-hide after 5 seconds
    setTimeout(() => {
      if (messageDisplay && messageDisplay.textContent === msg) {
        messageDisplay.textContent = "";
        messageDisplay.className = "topbar-message";
      }
    }, 5000);
  }
}

function setLoading(loading: boolean): void {
  state.operationState = loading ? "loading" : "idle";
  document.querySelectorAll(".topbar-btn").forEach((btn) => {
    (btn as HTMLButtonElement).disabled = loading;
  });
  if (container) {
    container.classList.toggle("loading", loading);
  }
}

function updateStatusDisplay(status: ops.StatusResponse | null): void {
  state.statusInfo = status;
  if (!statusDisplay) return;

  if (!status) {
    statusDisplay.innerHTML = '<span class="status-placeholder">No status</span>';
    return;
  }

  const kindStr =
    status.workspace.kind.type === "single_repo"
      ? "📁 Single Repo"
      : status.workspace.kind.type === "multi_repo"
        ? `📦 ${(status.workspace.kind as { repo_count: number }).repo_count} Repos`
        : "📂 Directory";

  const storeStr = status.store_exists
    ? `✅ .self (${status.manifest?.source_count || 0} files)`
    : "❌ Not synced";

  statusDisplay.innerHTML = `
    <span class="status-item" title="${status.workspace.root}">${kindStr} <strong>${status.workspace.name}</strong></span>
    <span class="status-divider">|</span>
    <span class="status-item">${storeStr}</span>
    ${status.manifest?.remote ? `<span class="status-divider">|</span><span class="status-item">🔗 ${status.manifest.remote.split("/").slice(-2).join("/")}</span>` : ""}
  `;
}

/** Format bytes to human readable size. */
export function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

// =============================================================================
// Operations
// =============================================================================

async function handleSync(force = false): Promise<void> {
  const path = pathInput?.value || ".";
  setLoading(true);
  setMessage("Syncing...");

  try {
    const result = await ops.syncQuery(path, { force });
    const fileCount = result.project.repositories.reduce(
      (sum, r) => sum + r.sources.length,
      0
    );
    setMessage(
      `✅ Synced: ${result.project.repositories.length} repos, ${fileCount} files`
    );
    // Refresh status
    await handleStatus(true);
    // Dispatch event for WASM to refresh
    window.dispatchEvent(new CustomEvent("vibe-graph-synced"));
  } catch (error) {
    setMessage(`❌ Sync failed: ${(error as Error).message}`, true);
  } finally {
    setLoading(false);
  }
}

async function handleGraph(force = false): Promise<void> {
  const path = pathInput?.value || ".";
  setLoading(true);
  setMessage("Building graph...");

  try {
    const result = await ops.buildGraphQuery(path, { force });
    setMessage(
      `✅ Graph: ${result.graph.nodes.length} nodes, ${result.graph.edges.length} edges${result.from_cache ? " (cached)" : ""}`
    );
    // Dispatch event for main.ts to refresh graph data
    window.dispatchEvent(new CustomEvent("vibe-graph-updated"));
  } catch (error) {
    setMessage(`❌ Graph failed: ${(error as Error).message}`, true);
  } finally {
    setLoading(false);
  }
}

async function handleStatus(silent = false): Promise<void> {
  const path = pathInput?.value || ".";
  if (!silent) {
    setLoading(true);
    setMessage("Getting status...");
  }

  try {
    const result = await ops.getStatus(path, { detailed: true });
    updateStatusDisplay(result);
    if (!silent) {
      setMessage(
        `✅ ${result.workspace.name}: ${result.store_exists ? "synced" : "not synced"}`
      );
    }
  } catch (error) {
    if (!silent) {
      setMessage(`❌ Status failed: ${(error as Error).message}`, true);
    }
    updateStatusDisplay(null);
  } finally {
    if (!silent) setLoading(false);
  }
}

async function handleLoad(): Promise<void> {
  const path = pathInput?.value || ".";
  setLoading(true);
  setMessage("Loading project...");

  try {
    const result = await ops.loadProject(path);
    const fileCount = result.project.repositories.reduce(
      (sum, r) => sum + r.sources.length,
      0
    );
    setMessage(
      `✅ Loaded: ${result.manifest.name} (${result.manifest.repo_count} repos, ${fileCount} files)`
    );
  } catch (error) {
    setMessage(`❌ Load failed: ${(error as Error).message}`, true);
  } finally {
    setLoading(false);
  }
}

async function handleClean(): Promise<void> {
  const path = pathInput?.value || ".";

  if (!confirm("Remove .self folder? This will delete all cached data.")) {
    return;
  }

  setLoading(true);
  setMessage("Cleaning...");

  try {
    const result = await ops.clean(path);
    setMessage(result.cleaned ? "✅ Cleaned .self folder" : "ℹ️ No .self folder to clean");
    await handleStatus(true);
  } catch (error) {
    setMessage(`❌ Clean failed: ${(error as Error).message}`, true);
  } finally {
    setLoading(false);
  }
}

async function handleGitChanges(): Promise<void> {
  const path = pathInput?.value || ".";
  setLoading(true);
  setMessage("Getting git changes...");

  try {
    const result = await ops.getGitChanges(path);
    const modified = result.changes.filter((c) => c.kind === "Modified").length;
    const added = result.changes.filter(
      (c) => c.kind === "Added" || c.kind === "Untracked"
    ).length;
    const deleted = result.changes.filter((c) => c.kind === "Deleted").length;

    let msg = `📝 Git: ${result.changes.length} changes`;
    if (modified) msg += ` (${modified} modified)`;
    if (added) msg += ` (${added} added)`;
    if (deleted) msg += ` (${deleted} deleted)`;

    setMessage(msg);
  } catch (error) {
    setMessage(`❌ Git changes failed: ${(error as Error).message}`, true);
  } finally {
    setLoading(false);
  }
}

// =============================================================================
// Render
// =============================================================================

function createButton(
  label: string,
  icon: string,
  onClick: () => void,
  title: string
): HTMLButtonElement {
  const btn = document.createElement("button");
  btn.className = "topbar-btn";
  btn.innerHTML = `<span class="btn-icon">${icon}</span><span class="btn-label">${label}</span>`;
  btn.title = title;
  btn.onclick = onClick;
  return btn;
}

export function render(targetId: string): void {
  const target = document.getElementById(targetId);
  if (!target) {
    console.error(`[TopBar] Target element #${targetId} not found`);
    return;
  }

  container = document.createElement("div");
  container.className = "topbar";
  container.innerHTML = `
    <div class="topbar-left">
      <div class="topbar-brand">
        <span class="brand-icon">◈</span>
        <span class="brand-name">Vibe Graph</span>
      </div>
    </div>
    <div class="topbar-center">
      <div class="topbar-path">
        <label for="path-input">Path:</label>
        <input type="text" id="path-input" value="." placeholder="." />
      </div>
      <div class="topbar-actions"></div>
    </div>
    <div class="topbar-right">
      <div class="topbar-status" id="topbar-status"></div>
      <div class="topbar-message" id="topbar-message"></div>
    </div>
  `;

  target.appendChild(container);

  // Get references
  pathInput = container.querySelector("#path-input");
  statusDisplay = container.querySelector("#topbar-status");
  messageDisplay = container.querySelector("#topbar-message");

  // Add action buttons
  const actionsContainer = container.querySelector(".topbar-actions");
  if (actionsContainer) {
    actionsContainer.appendChild(
      createButton("Sync", "🔄", () => handleSync(), "Scan and index codebase")
    );
    actionsContainer.appendChild(
      createButton("Graph", "📊", () => handleGraph(), "Build source code graph")
    );
    actionsContainer.appendChild(
      createButton("Status", "ℹ️", () => handleStatus(), "Show workspace status")
    );
    actionsContainer.appendChild(
      createButton("Load", "📂", () => handleLoad(), "Load from .self store")
    );
    actionsContainer.appendChild(
      createButton("Git", "📝", () => handleGitChanges(), "Show git changes")
    );
    actionsContainer.appendChild(
      createButton("Clean", "🗑️", () => handleClean(), "Remove .self folder")
    );
  }

  // Initial status fetch
  handleStatus(true);
}

// Export for external access
export { state, handleSync, handleGraph, handleStatus };

