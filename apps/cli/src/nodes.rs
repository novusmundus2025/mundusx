use crate::types::{Backend, NodeState, NodeStatus};

pub fn sample_nodes() -> Vec<NodeStatus> {
    vec![
        NodeStatus {
            node_id: "m-001".to_string(),
            backend: Backend::M,
            state: NodeState::Idle,
            available_memory_mb: 24_576,
            available_gpu_percent: 72,
            label: "MacBook M-series".to_string(),
            region: Some("local".to_string()),
        },
        NodeStatus {
            node_id: "m-002".to_string(),
            backend: Backend::M,
            state: NodeState::Busy,
            available_memory_mb: 16_384,
            available_gpu_percent: 44,
            label: "Studio M-series".to_string(),
            region: Some("local".to_string()),
        },
        NodeStatus {
            node_id: "cuda-001".to_string(),
            backend: Backend::Cuda,
            state: NodeState::Online,
            available_memory_mb: 49_152,
            available_gpu_percent: 84,
            label: "CUDA Worker".to_string(),
            region: Some("us-west".to_string()),
        },
        NodeStatus {
            node_id: "cuda-002".to_string(),
            backend: Backend::Cuda,
            state: NodeState::Offline,
            available_memory_mb: 32_768,
            available_gpu_percent: 0,
            label: "Dead CUDA Worker".to_string(),
            region: Some("us-east".to_string()),
        },
    ]
}

pub fn live_nodes() -> Vec<NodeStatus> {
    sample_nodes()
        .into_iter()
        .filter(|node| node.state != NodeState::Offline)
        .collect()
}
