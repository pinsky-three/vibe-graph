/**
 * Operations API client for vibe-graph-ops endpoints.
 *
 * These endpoints provide REST access to all vibe-graph operations,
 * mirroring the CLI commands.
 */

import type { ApiResponse } from "./types";

const BASE_URL = "/api/ops";

// =============================================================================
// Types
// =============================================================================

export type WorkspaceKind =
  | { type: "single_repo" }
  | { type: "multi_repo"; repo_count: number }
  | { type: "plain_directory" };

export interface WorkspaceInfo {
  name: string;
  root: string;
  kind: WorkspaceKind;
  repo_paths: string[];
}

export interface Source {
  path: string;
  relative_path: string;
  format: string;
  size: number | null;
  content: string | null;
}

export interface Repository {
  name: string;
  url: string;
  local_path: string;
  sources: Source[];
}

export interface Project {
  name: string;
  source: Record<string, unknown>;
  repositories: Repository[];
}

export interface Manifest {
  version: number;
  name: string;
  root: string;
  kind: string;
  last_sync: { secs_since_epoch: number; nanos_since_epoch: number };
  repo_count: number;
  source_count: number;
  total_size: number;
  remote: string | null;
}

// Response types
export interface SyncResponse {
  project: Project;
  workspace: WorkspaceInfo;
  path: string;
  remote: string | null;
  snapshot_created: string | null;
}

export interface GraphResponse {
  graph: {
    nodes: unknown[];
    edges: unknown[];
    metadata: Record<string, string>;
  };
  saved_path: string;
  output_path: string | null;
  from_cache: boolean;
}

export interface StatusResponse {
  workspace: WorkspaceInfo;
  store_exists: boolean;
  manifest: Manifest | null;
  snapshot_count: number;
  store_size: number;
  repositories: string[];
}

export interface LoadResponse {
  project: Project;
  manifest: Manifest;
}

export interface CleanResponse {
  cleaned: boolean;
  path: string;
}

export interface GitChangesResponse {
  changes: Array<{
    path: string;
    kind: string;
    staged: boolean;
  }>;
  repo_paths: string[];
}

export interface OpsError {
  code: string;
  message: string;
}

// =============================================================================
// API Helpers
// =============================================================================

async function opsGet<T>(endpoint: string): Promise<T> {
  const res = await fetch(`${BASE_URL}${endpoint}`);
  const json = (await res.json()) as ApiResponse<T | OpsError>;

  if (!res.ok) {
    const error = json.data as OpsError;
    throw new Error(error.message || `API error: ${res.status}`);
  }

  return json.data as T;
}

async function opsPost<T>(
  endpoint: string,
  body: Record<string, unknown>
): Promise<T> {
  const res = await fetch(`${BASE_URL}${endpoint}`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  const json = (await res.json()) as ApiResponse<T | OpsError>;

  if (!res.ok) {
    const error = json.data as OpsError;
    throw new Error(error.message || `API error: ${res.status}`);
  }

  return json.data as T;
}

async function opsDelete<T>(endpoint: string): Promise<T> {
  const res = await fetch(`${BASE_URL}${endpoint}`, { method: "DELETE" });
  const json = (await res.json()) as ApiResponse<T | OpsError>;

  if (!res.ok) {
    const error = json.data as OpsError;
    throw new Error(error.message || `API error: ${res.status}`);
  }

  return json.data as T;
}

// =============================================================================
// Operations API
// =============================================================================

/**
 * Sync a codebase (local path or GitHub).
 */
export async function sync(
  source: string,
  options: {
    ignore?: string[];
    noSave?: boolean;
    snapshot?: boolean;
    useCache?: boolean;
    force?: boolean;
  } = {}
): Promise<SyncResponse> {
  return opsPost<SyncResponse>("/sync", {
    source: { type: "local", path: source },
    ignore: options.ignore || [],
    no_save: options.noSave || false,
    snapshot: options.snapshot || false,
    use_cache: options.useCache || false,
    force: options.force || false,
  });
}

/**
 * Sync using query parameters (simpler for local paths).
 */
export async function syncQuery(
  source: string,
  options: {
    ignore?: string;
    noSave?: boolean;
    useCache?: boolean;
    force?: boolean;
  } = {}
): Promise<SyncResponse> {
  const params = new URLSearchParams({ source });
  if (options.ignore) params.set("ignore", options.ignore);
  if (options.noSave) params.set("no_save", "true");
  if (options.useCache) params.set("use_cache", "true");
  if (options.force) params.set("force", "true");

  return opsGet<SyncResponse>(`/sync?${params}`);
}

/**
 * Build source code graph.
 */
export async function buildGraph(
  path: string,
  options: { force?: boolean } = {}
): Promise<GraphResponse> {
  return opsPost<GraphResponse>("/graph", {
    path,
    output: null,
    force: options.force || false,
  });
}

/**
 * Build graph using query parameters.
 */
export async function buildGraphQuery(
  path: string,
  options: { force?: boolean } = {}
): Promise<GraphResponse> {
  const params = new URLSearchParams({ path });
  if (options.force) params.set("force", "true");

  return opsGet<GraphResponse>(`/graph?${params}`);
}

/**
 * Get workspace status.
 */
export async function getStatus(
  path: string,
  options: { detailed?: boolean } = {}
): Promise<StatusResponse> {
  const params = new URLSearchParams({ path });
  if (options.detailed) params.set("detailed", "true");

  return opsGet<StatusResponse>(`/status?${params}`);
}

/**
 * Load project from .self store.
 */
export async function loadProject(path: string): Promise<LoadResponse> {
  return opsGet<LoadResponse>(`/load?path=${encodeURIComponent(path)}`);
}

/**
 * Clean .self folder.
 */
export async function clean(path: string): Promise<CleanResponse> {
  return opsDelete<CleanResponse>(`/clean?path=${encodeURIComponent(path)}`);
}

/**
 * Get git changes for workspace.
 */
export async function getGitChanges(path: string): Promise<GitChangesResponse> {
  return opsGet<GitChangesResponse>(
    `/git-changes?path=${encodeURIComponent(path)}`
  );
}
