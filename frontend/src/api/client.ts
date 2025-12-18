/**
 * REST API client for vibe-graph-api.
 */

import type {
  ApiResponse,
  GitChangeSnapshot,
  GraphEdge,
  GraphNode,
  HealthResponse,
  SourceCodeGraph,
} from "./types";

const BASE_URL = "/api";

/**
 * Fetch with error handling.
 */
async function apiFetch<T>(endpoint: string): Promise<ApiResponse<T>> {
  const res = await fetch(`${BASE_URL}${endpoint}`);
  if (!res.ok) {
    throw new Error(`API error: ${res.status} ${res.statusText}`);
  }
  return res.json() as Promise<ApiResponse<T>>;
}

/**
 * Fetch the full source code graph.
 */
export async function fetchGraph(): Promise<SourceCodeGraph> {
  const response = await apiFetch<SourceCodeGraph>("/graph");
  return response.data;
}

/**
 * Fetch only graph nodes.
 */
export async function fetchNodes(): Promise<GraphNode[]> {
  const response = await apiFetch<GraphNode[]>("/graph/nodes");
  return response.data;
}

/**
 * Fetch only graph edges.
 */
export async function fetchEdges(): Promise<GraphEdge[]> {
  const response = await apiFetch<GraphEdge[]>("/graph/edges");
  return response.data;
}

/**
 * Fetch graph metadata.
 */
export async function fetchMetadata(): Promise<Record<string, string>> {
  const response = await apiFetch<Record<string, string>>("/graph/metadata");
  return response.data;
}

/**
 * Fetch current git change snapshot.
 */
export async function fetchGitChanges(): Promise<GitChangeSnapshot> {
  const response = await apiFetch<GitChangeSnapshot>("/git/changes");
  return response.data;
}

/**
 * Fetch health status.
 */
export async function fetchHealth(): Promise<HealthResponse> {
  const response = await apiFetch<HealthResponse>("/health");
  return response.data;
}
