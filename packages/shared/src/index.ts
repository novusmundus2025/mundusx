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

export interface NodeStatus extends NodeHeartbeat {
  label?: string;
  score?: number;
}

export interface RoutingDecision {
  requestId: string;
  selectedNodeId: string | null;
  selectedBackend: NodeBackend | null;
  reason: string;
}

export function isNodeAvailable(node: NodeStatus): boolean {
  return node.status === "online" || node.status === "idle";
}

export function scoreNodeForJob(node: NodeStatus, request: JobRequest): number {
  if (!isNodeAvailable(node)) {
    return -1;
  }

  if (request.preferredBackend && node.backend !== request.preferredBackend) {
    return -1;
  }

  const memoryScore = Math.min(node.availableMemoryMb / 1024, 32);
  const gpuScore = node.availableGpuPercent / 10;
  const statusScore = node.status === "idle" ? 10 : 4;
  const backendScore = node.backend === "m" ? 8 : 10;

  return memoryScore + gpuScore + statusScore + backendScore;
}

export function selectBestNode(
  nodes: NodeStatus[],
  request: JobRequest,
): RoutingDecision {
  let bestNode: NodeStatus | null = null;
  let bestScore = Number.NEGATIVE_INFINITY;

  for (const node of nodes) {
    const score = scoreNodeForJob(node, request);
    if (score > bestScore) {
      bestScore = score;
      bestNode = node;
    }
  }

  if (!bestNode || bestScore < 0) {
    return {
      requestId: request.requestId,
      selectedNodeId: null,
      selectedBackend: null,
      reason: "No live node matched the request",
    };
  }

  return {
    requestId: request.requestId,
    selectedNodeId: bestNode.nodeId,
    selectedBackend: bestNode.backend,
    reason: `Selected ${bestNode.label ?? bestNode.nodeId} with score ${bestScore.toFixed(2)}`,
  };
}
