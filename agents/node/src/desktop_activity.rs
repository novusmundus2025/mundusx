//! Bounded local job metadata. Never persist prompts, outputs or raw errors here.
use crate::contracts::{WorkerLaunchRequest, WorkerLaunchResponse};
use fs2::FileExt;
use serde_json::{json, Value};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

pub struct JobActivity {
    home: PathBuf,
    row: Value,
    started: Instant,
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

impl JobActivity {
    pub fn start(home: &Path, request: &WorkerLaunchRequest) -> Self {
        let row = json!({"record_id":uuid::Uuid::new_v4().to_string(),"job_id":request.job_id,
            "model":request.model,"operation":if request.prompt.starts_with("__MUNDUSX_OPENAI_TOOL_TURN_V1__") {"Generating tool calls"} else {"Generating model response"},
            "status":"running","started_at":now(),"updated_at":now(),"process_id":std::process::id()});
        let activity = Self {
            home: home.into(),
            row,
            started: Instant::now(),
        };
        let _ = persist(home, &activity.row);
        activity
    }
    pub fn finish(mut self, result: &Result<WorkerLaunchResponse, String>) {
        let completed = result.as_ref().is_ok_and(|v| v.status == "completed");
        self.row["status"] = json!(if completed { "completed" } else { "failed" });
        self.row["updated_at"] = json!(now());
        self.row["duration_ms"] = json!(self.started.elapsed().as_millis() as u64);
        if !completed {
            self.row["summary"] =
                json!("Worker execution failed. Check the local diagnostic log for details.");
        }
        let _ = persist(&self.home, &self.row);
    }
}
fn persist(home: &Path, row: &Value) -> std::io::Result<()> {
    fs::create_dir_all(home)?;
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let lock = options.open(home.join("desktop-jobs.lock"))?;
    lock.lock_exclusive()?;
    let path = home.join("desktop-jobs.json");
    let mut bytes = Vec::new();
    if let Ok(file) = fs::File::open(&path) {
        file.take(256001).read_to_end(&mut bytes)?;
    }
    let mut rows: Vec<Value> = if bytes.len() <= 256000 {
        serde_json::from_slice(&bytes).unwrap_or_default()
    } else {
        Vec::new()
    };
    rows.retain(|old| old["record_id"] != row["record_id"]);
    rows.push(row.clone());
    // Preserve active work even if many newer jobs finish in parallel.
    while rows.len() > 200 {
        let i = rows
            .iter()
            .position(|v| v["status"] != "running")
            .unwrap_or(0);
        rows.remove(i);
    }
    let temp = home.join("desktop-jobs.json.tmp");
    let mut file = options.truncate(true).open(&temp)?;
    file.write_all(&serde_json::to_vec(&rows)?)?;
    file.sync_all()?;
    drop(file);
    fs::rename(temp, path)?;
    FileExt::unlock(&lock)?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn job_lifecycle_records_failure_without_prompt_output_or_error_contents() {
        let home =
            std::env::temp_dir().join(format!("opengpu-private-activity-{}", uuid::Uuid::new_v4()));
        let request:WorkerLaunchRequest=serde_json::from_value(json!({"job_id":"fake-job","node_id":"fake-node","backend":"m","prompt":"PRIVATE-PROMPT-CANARY","model":"test-model","system_prompt":"PRIVATE-SYSTEM-CANARY"})).unwrap();
        let activity = JobActivity::start(&home, &request);
        let first = fs::read_to_string(home.join("desktop-jobs.json")).unwrap();
        assert!(first.contains("running"));
        assert!(!first.contains("PRIVATE-"));
        activity.finish(&Err("PRIVATE-ERROR-CANARY".into()));
        let last = fs::read_to_string(home.join("desktop-jobs.json")).unwrap();
        assert!(last.contains("failed"));
        assert!(last.contains("duration_ms"));
        assert!(!last.contains("PRIVATE-"));
        fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn concurrent_jobs_keep_separate_states_and_replace_their_own_rows() {
        let home = std::env::temp_dir().join(format!("opengpu-activity-{}", uuid::Uuid::new_v4()));
        std::thread::scope(|scope| {
            for i in 0..8 {
                let home = &home;
                scope.spawn(move || {
                    let mut row =
                        json!({"record_id":i,"job_id":format!("job-{i}"),"status":"running"});
                    persist(home, &row).unwrap();
                    row["status"] = json!(if i % 2 == 0 { "completed" } else { "failed" });
                    persist(home, &row).unwrap();
                });
            }
        });
        let rows: Vec<Value> =
            serde_json::from_slice(&fs::read(home.join("desktop-jobs.json")).unwrap()).unwrap();
        assert_eq!(rows.len(), 8);
        assert_eq!(
            rows.iter().filter(|v| v["status"] == "completed").count(),
            4
        );
        assert_eq!(rows.iter().filter(|v| v["status"] == "failed").count(), 4);
        fs::remove_dir_all(home).unwrap();
    }
}
