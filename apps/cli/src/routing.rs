use super::NodeStatus;

pub fn score_node(node: &NodeStatus) -> f64 {
    if node.status == "offline" {
        return -1.0;
    }

    let status_score = if node.status == "idle" { 10.0 } else { 4.0 };
    let memory_score = (node.available_memory_mb as f64 / 1024.0).min(32.0);
    let gpu_score = node.available_gpu_percent as f64 / 10.0;
    let backend_score = if node.backend == "m" { 8.0 } else { 10.0 };

    status_score + memory_score + gpu_score + backend_score
}

