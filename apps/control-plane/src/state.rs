use crate::contracts::{
    AgentRegistration, AgentState, Backend, ControlPlaneSnapshot, Heartbeat, JobClaimResponse,
    JobCompletion, JobRecord, JobRequest, JobStatus, NodeRecord,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ControlPlaneState {
    pub nodes: BTreeMap<String, NodeRecord>,
    pub jobs: BTreeMap<String, JobRecord>,
}

impl ControlPlaneState {
    pub fn snapshot(&self) -> serde_json::Value {
        let nodes: Vec<NodeRecord> = self.nodes.values().cloned().collect();
        let jobs: Vec<JobRecord> = self.jobs.values().cloned().collect();
        let online_count = nodes
            .iter()
            .filter(|node| node.state == AgentState::Ready || node.state == AgentState::Busy)
            .count();
        let paused_count = nodes
            .iter()
            .filter(|node| node.state == AgentState::Paused)
            .count();
        let policy_blocked_count = nodes
            .iter()
            .filter(|node| !node.policy_allowed)
            .count();
        let stopped_count = nodes
            .iter()
            .filter(|node| node.state == AgentState::Stopped)
            .count();
        let queued_job_count = jobs
            .iter()
            .filter(|job| job.status == JobStatus::Queued)
            .count();
        let assigned_job_count = jobs
            .iter()
            .filter(|job| job.status == JobStatus::Assigned)
            .count();
        let completed_job_count = jobs
            .iter()
            .filter(|job| job.status == JobStatus::Completed)
            .count();
        let failed_job_count = jobs
            .iter()
            .filter(|job| job.status == JobStatus::Failed)
            .count();

        serde_json::to_value(ControlPlaneSnapshot {
            nodes,
            jobs,
            online_count,
            paused_count,
            policy_blocked_count,
            stopped_count,
            queued_job_count,
            assigned_job_count,
            completed_job_count,
            failed_job_count,
        })
        .expect("snapshot json")
    }

    pub fn nodes_snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self.nodes.values().cloned().collect::<Vec<_>>())
            .expect("nodes json")
    }

    pub fn jobs_snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self.jobs.values().cloned().collect::<Vec<_>>())
            .expect("jobs json")
    }

    pub fn register(&mut self, registration: AgentRegistration) -> NodeRecord {
        let record = NodeRecord {
            node_id: registration.node_id.clone(),
            public_key_fingerprint: registration.public_key_fingerprint,
            public_key_hex: registration.public_key_hex,
            backend: registration.backend,
            contribution_percent: registration.contribution_percent,
            agent_version: registration.agent_version,
            state: AgentState::Starting,
            available_memory_mb: 0,
            available_gpu_percent: 0,
            power_source: "unknown".to_string(),
            on_battery: false,
            battery_percent: None,
            policy_allowed: false,
            policy_reason: None,
            updated_at: String::new(),
        };

        self.nodes.insert(registration.node_id, record.clone());
        record
    }

    pub fn submit_job(&mut self, request: JobRequest, submitted_at: String) -> JobRecord {
        let job_id = request.request_id.clone();
        let record = JobRecord {
            job_id: job_id.clone(),
            request_id: request.request_id,
            prompt: request.prompt,
            preferred_backend: request.preferred_backend,
            model: request.model,
            status: JobStatus::Queued,
            submitted_at,
            assigned_node_id: None,
            assigned_at: None,
            completed_at: None,
            worker_id: None,
            backend: None,
            output: None,
            error: None,
        };

        self.jobs.insert(job_id, record.clone());
        record
    }

    fn node_backend_matches(job: &JobRecord, node_backend: Backend) -> bool {
        job.preferred_backend == Backend::Auto
            || node_backend == Backend::Auto
            || job.preferred_backend == node_backend
    }

    pub fn claim_job(&mut self, node_id: &str, claimed_at: String) -> JobClaimResponse {
        let Some(node) = self.nodes.get(node_id) else {
            return JobClaimResponse { job: None };
        };

        if !(node.state == AgentState::Ready || node.state == AgentState::Busy) {
            return JobClaimResponse { job: None };
        }

        if !node.policy_allowed {
            return JobClaimResponse { job: None };
        }

        let node_backend = node.backend;
        let job_id = self
            .jobs
            .iter()
            .find(|(_, job)| {
                job.status == JobStatus::Queued && Self::node_backend_matches(job, node_backend)
            })
            .map(|(job_id, _)| job_id.clone());

        let Some(job_id) = job_id else {
            return JobClaimResponse { job: None };
        };

        if let Some(job) = self.jobs.get_mut(&job_id) {
            job.status = JobStatus::Assigned;
            job.assigned_node_id = Some(node_id.to_string());
            job.assigned_at = Some(claimed_at);
            job.backend = Some(node_backend);
            job.worker_id = None;
            job.output = None;
            job.error = None;

            if let Some(node) = self.nodes.get_mut(node_id) {
                node.state = AgentState::Busy;
                node.updated_at = job.assigned_at.clone().unwrap_or_default();
            }

            return JobClaimResponse {
                job: Some(job.clone()),
            };
        }

        JobClaimResponse { job: None }
    }

    pub fn complete_job(&mut self, completion: JobCompletion, completed_at: String) -> Option<JobRecord> {
        let updated_job = {
            let job = self.jobs.get_mut(&completion.job_id)?;
            if job.assigned_node_id.as_deref() != Some(completion.node_id.as_str()) {
                return Some(job.clone());
            }

            job.status = completion.status;
            job.worker_id = Some(completion.worker_id);
            job.backend = Some(completion.backend);
            job.output = completion.output;
            job.error = completion.error;
            job.completed_at = Some(completed_at.clone());
            job.clone()
        };

        if let Some(node) = self.nodes.get_mut(&completion.node_id) {
            if node.state == AgentState::Busy {
                node.state = AgentState::Ready;
            }
            node.backend = completion.backend;
            node.updated_at = completed_at;
        }

        Some(updated_job)
    }

    pub fn heartbeat(&mut self, heartbeat: Heartbeat, updated_at: String) -> NodeRecord {
        let record = NodeRecord {
            node_id: heartbeat.node_id.clone(),
            public_key_fingerprint: self
                .nodes
                .get(&heartbeat.node_id)
                .map(|node| node.public_key_fingerprint.clone())
                .unwrap_or_default(),
            public_key_hex: self
                .nodes
                .get(&heartbeat.node_id)
                .map(|node| node.public_key_hex.clone())
                .unwrap_or_default(),
            backend: heartbeat.backend,
            contribution_percent: heartbeat.contribution_percent,
            agent_version: self
                .nodes
                .get(&heartbeat.node_id)
                .map(|node| node.agent_version.clone())
                .unwrap_or_else(|| "0.1.0".to_string()),
            state: heartbeat.agent_state,
            available_memory_mb: heartbeat.available_memory_mb,
            available_gpu_percent: heartbeat.available_gpu_percent,
            power_source: heartbeat.power_source,
            on_battery: heartbeat.on_battery,
            battery_percent: heartbeat.battery_percent,
            policy_allowed: heartbeat.policy_allowed,
            policy_reason: heartbeat.policy_reason,
            updated_at: updated_at.clone(),
        };

        self.nodes.insert(heartbeat.node_id, record.clone());
        record
    }
}

pub fn state_path() -> PathBuf {
    std::env::var_os("OPENGPU_CONTROL_PLANE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("OPENGPU_HOME").map(PathBuf::from))
        .or_else(|| dirs::home_dir().map(|dir| dir.join(".opengpu-control-plane")))
        .unwrap_or_else(|| PathBuf::from(".opengpu-control-plane"))
        .join("state.json")
}

pub fn load_state() -> std::io::Result<Option<ControlPlaneState>> {
    let path = state_path();
    if !path.exists() {
        return Ok(None);
    }

    let raw = fs::read_to_string(path)?;
    let state: ControlPlaneState = serde_json::from_str(&raw)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    Ok(Some(state))
}

pub fn save_state(state: &ControlPlaneState) -> std::io::Result<PathBuf> {
    let path = state_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let data = serde_json::to_string_pretty(state).expect("state serialization");
    fs::write(&path, format!("{data}\n"))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready_state() -> ControlPlaneState {
        let mut state = ControlPlaneState::default();
        let registration = AgentRegistration {
            node_id: "node-1".to_string(),
            public_key_fingerprint: "fingerprint".to_string(),
            public_key_hex: "aabbcc".to_string(),
            backend: Backend::M,
            contribution_percent: 50,
            agent_version: "0.1.0".to_string(),
        };
        state.register(registration);
        state.heartbeat(
            Heartbeat {
                node_id: "node-1".to_string(),
                backend: Backend::M,
                agent_state: AgentState::Ready,
                available_memory_mb: 16_000,
                available_gpu_percent: 50,
                updated_at: "1".to_string(),
                contribution_percent: 50,
                power_source: "AC Power".to_string(),
                on_battery: false,
                battery_percent: Some(90),
                policy_allowed: true,
                policy_reason: None,
            },
            "1".to_string(),
        );
        state
    }

    #[test]
    fn claims_queued_job_for_ready_node() {
        let mut state = ready_state();
        state.submit_job(
            JobRequest {
                request_id: "job-1".to_string(),
                prompt: "hello world".to_string(),
                preferred_backend: Backend::Auto,
                model: None,
            },
            "2".to_string(),
        );

        let claim = state.claim_job("node-1", "3".to_string());
        let job = claim.job.expect("claimed job");
        assert_eq!(job.job_id, "job-1");
        assert_eq!(job.status, JobStatus::Assigned);
        assert_eq!(job.assigned_node_id.as_deref(), Some("node-1"));
    }

    #[test]
    fn completes_assigned_job() {
        let mut state = ready_state();
        state.submit_job(
            JobRequest {
                request_id: "job-1".to_string(),
                prompt: "hello world".to_string(),
                preferred_backend: Backend::Auto,
                model: None,
            },
            "2".to_string(),
        );

        let _ = state.claim_job("node-1", "3".to_string());
        let completed = state.complete_job(
            JobCompletion {
                job_id: "job-1".to_string(),
                node_id: "node-1".to_string(),
                worker_id: "worker-1".to_string(),
                backend: Backend::M,
                status: JobStatus::Completed,
                output: Some("done".to_string()),
                error: None,
            },
            "4".to_string(),
        )
        .expect("completed job");

        assert_eq!(completed.status, JobStatus::Completed);
        assert_eq!(completed.output.as_deref(), Some("done"));
    }
}
