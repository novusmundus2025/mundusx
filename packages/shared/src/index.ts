export type NodeBackend = "m" | "cuda";

export interface NodeCapability {
  backend: NodeBackend;
  label: string;
  available: boolean;
  score: number;
}

export interface NodeHeartbeat {
  nodeId: string;
  backend: NodeBackend;
  status: "online" | "busy" | "idle" | "offline";
  availableMemoryMb: number;
  availableGpuPercent: number;
  updatedAt: string;
}

export interface JobRequest {
  requestId: string;
  prompt: string;
  preferredBackend?: NodeBackend;
}

