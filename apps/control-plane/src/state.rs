use crate::contracts::{
    AgentRegistration, AgentState, Backend, ControlPlaneSnapshot, Heartbeat, JobClaimResponse,
    JobCompletion, JobEventRecord, JobRecord, JobRequest, JobStatus, NodeRecord,
    CreditsLedgerRecord,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ControlPlaneState {
    pub nodes: BTreeMap<String, NodeRecord>,
    pub jobs: BTreeMap<String, JobRecord>,
    pub job_events: Vec<JobEventRecord>,
    pub credits_ledger: Vec<CreditsLedgerRecord>,
}

impl ControlPlaneState {
    pub fn snapshot(&self, storage_source: &str) -> serde_json::Value {
        let nodes: Vec<NodeRecord> = self.nodes.values().cloned().collect();
        let jobs: Vec<JobRecord> = self.jobs.values().cloned().collect();
        let job_events = self.job_events.len();
        let credits_ledger = self.credits_ledger.len();
        let credits_total = self.credits_total();
        let credits_by_node = self.credits_by_node();
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
            job_events,
            credits_ledger,
            credits_total,
            credits_by_node,
            storage_source: storage_source.to_string(),
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

    pub fn job_events_snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self.job_events.clone()).expect("job events json")
    }

    pub fn credits_snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "ledger": self.credits_ledger.clone(),
            "total": self.credits_total(),
            "by_node": self.credits_by_node(),
        })
    }

    pub fn record_job_event(
        &mut self,
        node_id: Option<String>,
        job_id: Option<String>,
        event_type: impl Into<String>,
        payload: serde_json::Value,
        created_at: String,
    ) -> JobEventRecord {
        let record = JobEventRecord {
            id: self.job_events.len() as u64 + 1,
            node_id,
            job_id,
            event_type: event_type.into(),
            payload,
            created_at,
        };
        self.job_events.push(record.clone());
        record
    }

    pub fn record_credit_award(
        &mut self,
        device_id: Option<String>,
        job_id: Option<String>,
        amount: f64,
        currency: &str,
        metadata: serde_json::Value,
        created_at: String,
    ) -> Option<CreditsLedgerRecord> {
        let job_id_ref = job_id.as_deref();
        if self
            .credits_ledger
            .iter()
            .any(|entry| entry.entry_type == "job_reward" && entry.job_id.as_deref() == job_id_ref)
        {
            return None;
        }

        let record = CreditsLedgerRecord {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: None,
            device_id,
            job_id,
            entry_type: "job_reward".to_string(),
            amount,
            currency: currency.to_string(),
            metadata,
            created_at,
        };
        self.credits_ledger.push(record.clone());
        Some(record)
    }

    pub fn credits_total(&self) -> f64 {
        let total = self.credits_ledger.iter().map(|entry| entry.amount).sum::<f64>();
        normalize_amount(total)
    }

    pub fn credits_by_node(&self) -> BTreeMap<String, f64> {
        let mut by_node = BTreeMap::new();
        for entry in &self.credits_ledger {
            if let Some(device_id) = entry.device_id.as_ref() {
                *by_node.entry(device_id.clone()).or_insert(0.0) += entry.amount;
            }
        }
        for value in by_node.values_mut() {
            *value = normalize_amount(*value);
        }
        by_node
    }

    pub fn award_job_reward(
        &mut self,
        job: &JobRecord,
        completed_at: String,
    ) -> Option<CreditsLedgerRecord> {
        let node_id = job.assigned_node_id.clone()?;
        let node = self.nodes.get(&node_id)?;
        let prompt_chars = job.prompt.chars().count() as f64;
        let output_chars = job.output.as_ref().map(|output| output.chars().count() as f64).unwrap_or(0.0);
        let work_units = ((prompt_chars + output_chars) / 400.0).ceil().max(1.0);
        let contribution_multiplier = 1.0 + (node.contribution_percent as f64 / 100.0);
        let amount = ((work_units * contribution_multiplier) * 100.0).round() / 100.0;
        let metadata = serde_json::json!({
            "formula": "ceil((prompt_chars + output_chars) / 400) * (1 + contribution_percent / 100)",
            "prompt_chars": prompt_chars,
            "output_chars": output_chars,
            "contribution_percent": node.contribution_percent,
            "backend": node.backend,
            "job_status": job.status,
        });
        self.record_credit_award(
            Some(node_id),
            Some(job.job_id.clone()),
            amount,
            "credits",
            metadata,
            completed_at,
        )
    }

    pub fn register(&mut self, registration: AgentRegistration) -> NodeRecord {
        let record = NodeRecord {
            node_id: registration.node_id.clone(),
            public_key_fingerprint: registration.public_key_fingerprint,
            public_key_hex: registration.public_key_hex,
            hostname: registration.hostname,
            identity_trust_path: registration.identity_trust_path,
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
            worker_health: None,
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
            system_prompt: request.system_prompt,
            max_tokens: request.max_tokens,
            temperature: request.temperature,
            top_p: request.top_p,
            seed: request.seed,
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
            hostname: self
                .nodes
                .get(&heartbeat.node_id)
                .map(|node| node.hostname.clone())
                .unwrap_or_default(),
            identity_trust_path: heartbeat.identity_trust_path.clone(),
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
            worker_health: Some(heartbeat.worker_health),
            updated_at: updated_at.clone(),
        };

        self.nodes.insert(heartbeat.node_id, record.clone());
        record
    }
}

fn normalize_amount(value: f64) -> f64 {
    let rounded = (value * 100.0).round() / 100.0;
    if rounded.abs() < 0.005 {
        0.0
    } else {
        rounded
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
    use crate::contracts::WorkerHealthReport;

    fn ready_state() -> ControlPlaneState {
        let mut state = ControlPlaneState::default();
        let registration = AgentRegistration {
            node_id: "node-1".to_string(),
            public_key_fingerprint: "fingerprint".to_string(),
            public_key_hex: "aabbcc".to_string(),
            hostname: "host-1".to_string(),
            identity_trust_path: "local-encrypted-fallback".to_string(),
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
                hostname: "host-1".to_string(),
                identity_trust_path: "local-encrypted-fallback".to_string(),
                power_source: "AC Power".to_string(),
                on_battery: false,
                battery_percent: Some(90),
                policy_allowed: true,
                policy_reason: None,
                worker_health: WorkerHealthReport {
                    healthy: true,
                    model_dir: "/tmp/models".to_string(),
                    model_name: Some("demo".to_string()),
                    model_path: Some("/tmp/models/demo.gguf".to_string()),
                    llama_cli_available: true,
                    blas_device_available: true,
                    power_source: "AC Power".to_string(),
                    on_battery: false,
                    battery_percent: Some(90),
                    runtime_mode: "batch".to_string(),
                    checked_at: "1".to_string(),
                    notes: vec![],
                },
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
                system_prompt: Some("You are a terse assistant.".to_string()),
                max_tokens: Some(32),
                temperature: Some(0.2),
                top_p: Some(0.9),
                seed: Some(42),
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
    fn stores_worker_health_snapshot_on_heartbeat() {
        let state = ready_state();
        let node = state.nodes.get("node-1").expect("node exists");
        let worker_health = node.worker_health.as_ref().expect("worker health present");
        assert!(worker_health.healthy);
        assert_eq!(worker_health.model_name.as_deref(), Some("demo"));
        assert_eq!(worker_health.model_path.as_deref(), Some("/tmp/models/demo.gguf"));
        assert!(worker_health.llama_cli_available);
        assert!(worker_health.blas_device_available);
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
                system_prompt: None,
                max_tokens: None,
                temperature: None,
                top_p: None,
                seed: None,
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
