use crate::types::{Backend, JobRequest, NodeState, NodeStatus, RoutingDecision};

pub fn score_node(node: &NodeStatus, preferred_backend: Backend) -> f64 {
    if node.state == NodeState::Offline {
        return -1.0;
    }

    if !preferred_backend.is_auto() && node.backend != preferred_backend {
        return -1.0;
    }

    let state_score = match node.state {
        NodeState::Idle => 12.0,
        NodeState::Online => 8.0,
        NodeState::Busy => 3.0,
        NodeState::Offline => -1.0,
    };
    let memory_score = (node.available_memory_mb as f64 / 1024.0).min(32.0);
    let gpu_score = node.available_gpu_percent as f64 / 10.0;
    let backend_score = match node.backend {
        Backend::M => 8.0,
        Backend::Cuda => 0.0,
        Backend::Auto => 0.0,
    };

    state_score + memory_score + gpu_score + backend_score
}

pub fn select_best_node(nodes: &[NodeStatus], request: &JobRequest) -> RoutingDecision {
    let mut best_node: Option<&NodeStatus> = None;
    let mut best_score = f64::NEG_INFINITY;

    for node in nodes {
        let score = score_node(node, request.preferred_backend);
        if score > best_score {
            best_score = score;
            best_node = Some(node);
        }
    }

    match best_node {
        Some(node) if best_score >= 0.0 => RoutingDecision {
            request_id: request.request_id.clone(),
            selected_node_id: Some(node.node_id.clone()),
            selected_backend: Some(node.backend),
            reason: format!("selected {} with score {:.2}", node.label, best_score),
        },
        _ => RoutingDecision {
            request_id: request.request_id.clone(),
            selected_node_id: None,
            selected_backend: None,
            reason: "no live node matched the request".to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, backend: Backend, state: NodeState, memory: u32, gpu: u32) -> NodeStatus {
        NodeStatus {
            node_id: id.to_string(),
            backend,
            state,
            available_memory_mb: memory,
            available_gpu_percent: gpu,
            label: id.to_string(),
            region: None,
        }
    }

    #[test]
    fn prefers_matching_backend_when_requested() {
        let nodes = vec![
            node("m-1", Backend::M, NodeState::Idle, 12_288, 40),
        ];
        let request = JobRequest {
            request_id: "req-1".to_string(),
            prompt: "hello".to_string(),
            preferred_backend: Backend::M,
        };

        let decision = select_best_node(&nodes, &request);

        assert_eq!(decision.selected_node_id.as_deref(), Some("m-1"));
        assert_eq!(decision.selected_backend, Some(Backend::M));
    }

    #[test]
    fn skips_offline_nodes() {
        let nodes = vec![
            node("offline", Backend::M, NodeState::Offline, 65_536, 100),
            node("live", Backend::M, NodeState::Online, 32_768, 60),
        ];
        let request = JobRequest {
            request_id: "req-2".to_string(),
            prompt: "status".to_string(),
            preferred_backend: Backend::Auto,
        };

        let decision = select_best_node(&nodes, &request);

        assert_eq!(decision.selected_node_id.as_deref(), Some("live"));
    }
}
