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
    ]
}

pub fn live_nodes() -> Vec<NodeStatus> {
    sample_nodes()
        .into_iter()
        .filter(|node| node.state != NodeState::Offline)
        .collect()
}
