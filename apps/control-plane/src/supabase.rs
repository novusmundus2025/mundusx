use crate::contracts::{AgentRegistration, Heartbeat, JobCompletion, JobRecord, JobStatus};
use serde_json::Value;
use std::env;
use std::io::Write;
use std::process::{Command, Stdio};

pub struct SupabaseMirror {
    database_url: String,
}

impl SupabaseMirror {
    pub fn from_env() -> Option<Self> {
        let database_url = env::var("DATABASE_URL").ok()?;
        if database_url.contains("[YOUR-PASSWORD]") || database_url.contains("YOUR-PASSWORD") {
            return None;
        }
        Some(Self { database_url })
    }

    pub fn startup_status() -> &'static str {
        match env::var("DATABASE_URL") {
            Ok(value) if value.contains("[YOUR-PASSWORD]") || value.contains("YOUR-PASSWORD") => {
                "placeholder"
            }
            Ok(_) => {
                if Self::psql_available() {
                    "configured"
                } else {
                    "configured (psql missing)"
                }
            }
            Err(_) => "not configured",
        }
    }

    fn psql_available() -> bool {
        Command::new("sh")
            .args(["-lc", "command -v psql >/dev/null 2>&1"])
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }

    fn quote(value: &str) -> String {
        format!("'{}'", value.replace('\\', "\\\\").replace('\'', "''"))
    }

    fn quote_opt(value: Option<&str>) -> String {
        value.map(Self::quote).unwrap_or_else(|| "NULL".to_string())
    }

    fn bool_sql(value: bool) -> &'static str {
        if value {
            "TRUE"
        } else {
            "FALSE"
        }
    }

    fn int_sql<T: ToString>(value: T) -> String {
        value.to_string()
    }

    fn ts_expr(epoch: &str) -> String {
        if epoch.parse::<f64>().is_ok() {
            format!("to_timestamp({epoch})")
        } else {
            "now()".to_string()
        }
    }

    fn json_sql(value: &Value) -> String {
        Self::quote(&value.to_string()) + "::jsonb"
    }

    fn run_sql(&self, sql: &str) -> Result<(), String> {
        if !Self::psql_available() {
            return Err("psql client is not installed".to_string());
        }

        let mut child = Command::new("psql")
            .env("DATABASE_URL", &self.database_url)
            .args(["-v", "ON_ERROR_STOP=1", "-X", "-qAt", "-f", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| error.to_string())?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(sql.as_bytes())
                .map_err(|error| error.to_string())?;
        }

        let output = child.wait_with_output().map_err(|error| error.to_string())?;
        if output.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
        }
    }

    pub fn ensure_schema(&self) -> Result<(), String> {
        self.run_sql(
            r#"
CREATE TABLE IF NOT EXISTS devices (
    node_id text PRIMARY KEY,
    public_key_fingerprint text UNIQUE NOT NULL,
    user_id text,
    backend text NOT NULL,
    contribution_percent integer NOT NULL,
    power_source text NOT NULL,
    on_battery boolean NOT NULL,
    battery_percent integer,
    policy_allowed boolean NOT NULL,
    policy_reason text,
    agent_version text NOT NULL,
    state text NOT NULL,
    last_seen_at timestamptz NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS heartbeats (
    id bigserial PRIMARY KEY,
    node_id text NOT NULL REFERENCES devices(node_id) ON DELETE CASCADE,
    backend text NOT NULL,
    agent_state text NOT NULL,
    available_memory_mb integer NOT NULL,
    available_gpu_percent integer NOT NULL,
    contribution_percent integer NOT NULL,
    power_source text NOT NULL,
    on_battery boolean NOT NULL,
    battery_percent integer,
    policy_allowed boolean NOT NULL,
    policy_reason text,
    updated_at timestamptz NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS jobs (
    job_id text PRIMARY KEY,
    request_id text UNIQUE NOT NULL,
    user_id text,
    prompt text NOT NULL,
    preferred_backend text NOT NULL,
    model text,
    status text NOT NULL,
    assigned_node_id text REFERENCES devices(node_id),
    worker_id text,
    backend text,
    output text,
    error text,
    submitted_at timestamptz NOT NULL,
    assigned_at timestamptz,
    completed_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS job_events (
    id bigserial PRIMARY KEY,
    job_id text NOT NULL REFERENCES jobs(job_id) ON DELETE CASCADE,
    event_type text NOT NULL,
    payload jsonb NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS policy_rules (
    id bigserial PRIMARY KEY,
    name text UNIQUE NOT NULL,
    enabled boolean NOT NULL,
    rule_type text NOT NULL,
    rule_config jsonb NOT NULL,
    description text,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS credits_ledger (
    id bigserial PRIMARY KEY,
    user_id text,
    device_id text,
    job_id text,
    entry_type text NOT NULL,
    amount numeric NOT NULL,
    currency text NOT NULL,
    metadata jsonb NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);
"#,
        )
    }

    pub fn record_registration(&self, registration: &AgentRegistration) -> Result<(), String> {
        let sql = format!(
            r#"
INSERT INTO devices (
    node_id, public_key_fingerprint, backend, contribution_percent, power_source,
    on_battery, battery_percent, policy_allowed, policy_reason, agent_version,
    state, last_seen_at, updated_at
) VALUES (
    {node_id}, {fingerprint}, {backend}, {contribution}, 'unknown',
    FALSE, NULL, TRUE, NULL, {version},
    'starting', now(), now()
)
ON CONFLICT (node_id) DO UPDATE SET
    public_key_fingerprint = EXCLUDED.public_key_fingerprint,
    backend = EXCLUDED.backend,
    contribution_percent = EXCLUDED.contribution_percent,
    agent_version = EXCLUDED.agent_version,
    state = EXCLUDED.state,
    updated_at = now();
"#,
            node_id = Self::quote(&registration.node_id),
            fingerprint = Self::quote(&registration.public_key_fingerprint),
            backend = Self::quote(registration.backend.as_str()),
            contribution = Self::int_sql(registration.contribution_percent),
            version = Self::quote(&registration.agent_version),
        );
        self.run_sql(&sql)
    }

    pub fn record_heartbeat(&self, heartbeat: &Heartbeat) -> Result<(), String> {
        let sql = format!(
            r#"
INSERT INTO devices (
    node_id, public_key_fingerprint, backend, contribution_percent, power_source,
    on_battery, battery_percent, policy_allowed, policy_reason, agent_version,
    state, last_seen_at, updated_at
) VALUES (
    {node_id}, COALESCE((SELECT public_key_fingerprint FROM devices WHERE node_id = {node_id}), ''),
    {backend}, {contribution}, {power_source},
    {on_battery}, {battery_percent}, {policy_allowed}, {policy_reason}, COALESCE((SELECT agent_version FROM devices WHERE node_id = {node_id}), '0.1.0'),
    {agent_state}, {updated_at}, now()
)
ON CONFLICT (node_id) DO UPDATE SET
    backend = EXCLUDED.backend,
    contribution_percent = EXCLUDED.contribution_percent,
    power_source = EXCLUDED.power_source,
    on_battery = EXCLUDED.on_battery,
    battery_percent = EXCLUDED.battery_percent,
    policy_allowed = EXCLUDED.policy_allowed,
    policy_reason = EXCLUDED.policy_reason,
    state = EXCLUDED.state,
    last_seen_at = EXCLUDED.last_seen_at,
    updated_at = now();

INSERT INTO heartbeats (
    node_id, backend, agent_state, available_memory_mb, available_gpu_percent,
    contribution_percent, power_source, on_battery, battery_percent,
    policy_allowed, policy_reason, updated_at
) VALUES (
    {node_id}, {backend}, {agent_state}, {available_memory_mb}, {available_gpu_percent},
    {contribution}, {power_source}, {on_battery}, {battery_percent},
    {policy_allowed}, {policy_reason}, {updated_at_ts}
);
"#,
            node_id = Self::quote(&heartbeat.node_id),
            backend = Self::quote(heartbeat.backend.as_str()),
            contribution = Self::int_sql(heartbeat.contribution_percent),
            power_source = Self::quote(&heartbeat.power_source),
            on_battery = Self::bool_sql(heartbeat.on_battery),
            battery_percent = heartbeat
                .battery_percent
                .map(Self::int_sql)
                .unwrap_or_else(|| "NULL".to_string()),
            policy_allowed = Self::bool_sql(heartbeat.policy_allowed),
            policy_reason = Self::quote_opt(heartbeat.policy_reason.as_deref()),
            agent_state = Self::quote(heartbeat.agent_state.as_str()),
            available_memory_mb = Self::int_sql(heartbeat.available_memory_mb),
            available_gpu_percent = Self::int_sql(heartbeat.available_gpu_percent),
            updated_at = Self::ts_expr(&heartbeat.updated_at),
            updated_at_ts = Self::ts_expr(&heartbeat.updated_at),
        );
        self.run_sql(&sql)
    }

    pub fn record_job(&self, job: &JobRecord) -> Result<(), String> {
        let sql = format!(
            r#"
INSERT INTO jobs (
    job_id, request_id, prompt, preferred_backend, model, status,
    assigned_node_id, worker_id, backend, output, error,
    submitted_at, assigned_at, completed_at, updated_at
) VALUES (
    {job_id}, {request_id}, {prompt}, {preferred_backend}, {model}, {status},
    {assigned_node_id}, {worker_id}, {backend}, {output}, {error},
    {submitted_at}, {assigned_at}, {completed_at}, now()
)
ON CONFLICT (job_id) DO UPDATE SET
    request_id = EXCLUDED.request_id,
    prompt = EXCLUDED.prompt,
    preferred_backend = EXCLUDED.preferred_backend,
    model = EXCLUDED.model,
    status = EXCLUDED.status,
    assigned_node_id = EXCLUDED.assigned_node_id,
    worker_id = EXCLUDED.worker_id,
    backend = EXCLUDED.backend,
    output = EXCLUDED.output,
    error = EXCLUDED.error,
    submitted_at = EXCLUDED.submitted_at,
    assigned_at = EXCLUDED.assigned_at,
    completed_at = EXCLUDED.completed_at,
    updated_at = now();
"#,
            job_id = Self::quote(&job.job_id),
            request_id = Self::quote(&job.request_id),
            prompt = Self::quote(&job.prompt),
            preferred_backend = Self::quote(job.preferred_backend.as_str()),
            model = Self::quote_opt(job.model.as_deref()),
            status = Self::quote(match job.status {
                JobStatus::Queued => "queued",
                JobStatus::Assigned => "assigned",
                JobStatus::Completed => "completed",
                JobStatus::Failed => "failed",
            }),
            assigned_node_id = Self::quote_opt(job.assigned_node_id.as_deref()),
            worker_id = Self::quote_opt(job.worker_id.as_deref()),
            backend = Self::quote_opt(job.backend.map(|backend| backend.as_str())),
            output = Self::quote_opt(job.output.as_deref()),
            error = Self::quote_opt(job.error.as_deref()),
            submitted_at = Self::ts_expr(&job.submitted_at),
            assigned_at = job
                .assigned_at
                .as_deref()
                .map(Self::ts_expr)
                .unwrap_or_else(|| "NULL".to_string()),
            completed_at = job
                .completed_at
                .as_deref()
                .map(Self::ts_expr)
                .unwrap_or_else(|| "NULL".to_string()),
        );
        self.run_sql(&sql)
    }

    pub fn record_job_completion(&self, completion: &JobCompletion, job: &JobRecord) -> Result<(), String> {
        self.record_job(job)?;
        let payload = serde_json::to_value(completion).unwrap_or(Value::Null);
        self.record_job_event(&completion.job_id, "completion", &payload)
    }

    pub fn record_job_event(
        &self,
        job_id: &str,
        event_type: &str,
        payload: &Value,
    ) -> Result<(), String> {
        let sql = format!(
            r#"
INSERT INTO job_events (job_id, event_type, payload)
VALUES ({job_id}, {event_type}, {payload});
"#,
            job_id = Self::quote(job_id),
            event_type = Self::quote(event_type),
            payload = Self::json_sql(payload),
        );
        self.run_sql(&sql)
    }
}
