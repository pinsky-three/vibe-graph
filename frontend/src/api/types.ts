/**
 * API types matching the Rust vibe-graph-core and vibe-graph-api DTOs.
 */

// Node ID is a tuple with single u64 in JSON
export type NodeId = [number];

// Edge ID is a tuple with single u64 in JSON
export type EdgeId = [number];

export type GraphNodeKind =
  | "Module"
  | "File"
  | "Directory"
  | "Service"
  | "Test"
  | "Other";

export interface GraphNode {
  id: NodeId;
  name: string;
  kind: GraphNodeKind;
  metadata: Record<string, string>;
}

export interface GraphEdge {
  id: EdgeId;
  from: NodeId;
  to: NodeId;
  relationship: string;
  metadata: Record<string, string>;
}

export interface SourceCodeGraph {
  nodes: GraphNode[];
  edges: GraphEdge[];
  metadata: Record<string, string>;
}

export type GitChangeKind =
  | "Modified"
  | "Added"
  | "Deleted"
  | "RenamedFrom"
  | "RenamedTo"
  | "Untracked";

export interface GitFileChange {
  path: string;
  kind: GitChangeKind;
  staged: boolean;
}

export interface GitChangeSnapshot {
  changes: GitFileChange[];
}

export interface HealthResponse {
  status: string;
  nodes: number;
  edges: number;
}

export interface ApiResponse<T> {
  data: T;
  timestamp: number;
}

// WebSocket message types
export type WsServerMessage =
  | { type: "git_changes"; data: GitChangeSnapshot }
  | { type: "graph_updated"; node_count: number; edge_count: number }
  | { type: "error"; code: string; message: string }
  | { type: "pong" };

export type WsClientMessage =
  | { type: "subscribe"; topics: string[] }
  | { type: "ping" };
