//! Local GPU handoff: the helper holds media.lock while the agent drains LLM work.
use std::{fs, path::Path};

fn locked(path: &Path) -> bool {
    let Ok(file) = fs::OpenOptions::new().read(true).write(true).open(path) else { return false; };
    matches!(file.try_lock(), Err(std::fs::TryLockError::WouldBlock))
}

pub fn process_stamp(pid: u32) -> Option<String> {
    let raw = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let mut fields = raw.rsplit_once(") ")?.1.split_whitespace();
    if matches!(fields.next()?, "Z" | "X") { return None; }
    fields.nth(18).map(str::to_owned)
}

pub fn live_owner(home: &Path) -> Option<String> {
    let media = home.join("media");
    let value: serde_json::Value = serde_json::from_slice(&fs::read(media.join("lifecycle/lease.json")).ok()?).ok()?;
    let owner = value["owner"].as_str()?;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    if owner.len() != 32 || !owner.bytes().all(|b| b.is_ascii_hexdigit())
        || value["expires_at"].as_u64()? <= now || !locked(&media.join("worker.lock")) { return None; }
    if cfg!(target_os = "linux") {
        let pid = u32::try_from(value["pid"].as_u64()?).ok()?;
        if process_stamp(pid)?.as_str() != value["start_ticks"].as_str()? { return None; }
    }
    Some(owner.into())
}

pub fn resident(home: &Path) -> bool {
    let Some(owner) = live_owner(home) else { return false; };
    let Ok(raw) = fs::read(home.join("media/active-container.json")) else { return false; };
    let Ok(marker) = serde_json::from_slice::<serde_json::Value>(&raw) else { return false; };
    marker["persistent_owner"].as_str() == Some(owner.as_str())
}

/// A live helper owns the shared GPU slot; a leftover container alone is recovery.
pub fn active(home: &Path) -> bool {
    let media = home.join("media");
    if locked(&media.join("worker.lock")) && locked(&media.join("job.lock")) { return true; }
    if live_owner(home).is_some() && locked(&media.join("job.lock")) { return true; }
    let Ok(bytes) = fs::read(media.join("drain-request.json")) else { return false; };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else { return false; };
    let Some(id) = value["id"].as_str() else { return false; };
    if id.len() != 32 || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) { return false; }
    let Ok(file) = fs::OpenOptions::new().read(true).write(true).open(media.join("media.lock")) else { return false; };
    matches!(file.try_lock(), Err(std::fs::TryLockError::WouldBlock))
}

pub fn request(home: &Path) -> Option<String> {
    let media = home.join("media");
    // A crashed helper may leave its container using the GPU. Fail closed until
    // the helper confirms that its owned container has stopped.
    if media.join("active-container.json").exists() && !resident(home) {
        return Some("container-recovery-required".to_string());
    }
    // A serving job also owns job.lock. Its helper's concrete handoff token
    // must take precedence over the generic active-job marker, or the helper
    // waits forever for an acknowledgement of a different ID.
    if let Some(id) = fs::read(media.join("drain-request.json")).ok()
        .and_then(|raw| serde_json::from_slice::<serde_json::Value>(&raw).ok())
        .and_then(|value| value["id"].as_str().map(str::to_string)) {
        if id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit()) && locked(&media.join("media.lock")) {
            return Some(id);
        }
    }
    if locked(&media.join("worker.lock")) && locked(&media.join("job.lock")) { return Some("media-job-active".into()); }
    if resident(home) && !locked(&media.join("media.lock")) { return Some("media-runtime-resident".into()); }
    if live_owner(home).is_some() && locked(&media.join("job.lock")) && !locked(&media.join("media.lock")) {
        return Some("media-job-active".into());
    }
    None
}

pub fn acknowledge(home: &Path, id: &str) -> Result<(), String> {
    let path = home.join("media/drain-ack.json");
    let tmp = home.join(format!("media/drain-ack-{id}.tmp"));
    fs::write(&tmp, serde_json::json!({"id":id,"drained":true}).to_string()).map_err(|e|e.to_string())?;
    fs::rename(tmp, path).map_err(|e|e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn helper_handoff_token_precedes_the_serving_job_marker() {
        let home=std::env::temp_dir().join(format!("handoff-{}",uuid::Uuid::new_v4()));
        let media=home.join("media");fs::create_dir_all(&media).unwrap();
        let open=|name:&str|fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(media.join(name)).unwrap();
        let worker=open("worker.lock");worker.lock().unwrap();
        let job=open("job.lock");job.lock().unwrap();
        let helper=open("media.lock");helper.lock().unwrap();
        let id="0123456789abcdef0123456789abcdef";
        fs::write(media.join("drain-request.json"),serde_json::json!({"id":id}).to_string()).unwrap();
        assert_eq!(request(&home).as_deref(),Some(id));
        drop(helper);
        assert_eq!(request(&home).as_deref(),Some("media-job-active"));
        drop(job);drop(worker);fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn native_image_job_reserves_slot_without_a_container_lease() {
        let home=std::env::temp_dir().join(format!("native-drain-{}",uuid::Uuid::new_v4()));
        let media=home.join("media");fs::create_dir_all(&media).unwrap();
        let worker=fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(media.join("worker.lock")).unwrap();worker.lock().unwrap();
        let job=fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(media.join("job.lock")).unwrap();job.lock().unwrap();
        assert!(active(&home));assert_eq!(request(&home).as_deref(),Some("media-job-active"));
        drop(job);assert!(!active(&home));assert!(request(&home).is_none());drop(worker);fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn resident_runtime_reserves_memory_but_only_a_job_occupies_the_slot() {
        let home = std::env::temp_dir().join(format!("opengpu-warm-{}", uuid::Uuid::new_v4()));
        let media = home.join("media");
        fs::create_dir_all(media.join("lifecycle")).unwrap();
        let owner = "0123456789abcdef0123456789abcdef";
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        fs::write(media.join("lifecycle/lease.json"), serde_json::json!({"owner":owner,"expires_at":now+120,
            "pid":std::process::id(),"start_ticks":process_stamp(std::process::id())}).to_string()).unwrap();
        fs::write(media.join("active-container.json"), serde_json::json!({"persistent_owner":owner}).to_string()).unwrap();
        let worker = fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(media.join("worker.lock")).unwrap();
        worker.lock().unwrap();
        assert!(resident(&home));
        assert!(!active(&home));
        assert_eq!(request(&home).as_deref(), Some("media-runtime-resident"));
        let job = fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(media.join("job.lock")).unwrap();
        job.lock().unwrap();
        assert!(active(&home));
        drop(job);
        assert!(!active(&home));
        fs::write(media.join("lifecycle/lease.json"), serde_json::json!({"owner":owner,"expires_at":0}).to_string()).unwrap();
        assert!(!resident(&home));
        assert_eq!(request(&home).as_deref(), Some("container-recovery-required"));
        drop(worker);
        assert!(live_owner(&home).is_none());
        fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn stale_request_without_live_helper_is_ignored() {
        let home = std::env::temp_dir().join(format!("opengpu-drain-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(home.join("media")).unwrap();
        let id = "0123456789abcdef0123456789abcdef";
        fs::write(home.join("media/drain-request.json"), serde_json::json!({"id":id}).to_string()).unwrap();
        let lock = fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(home.join("media/media.lock")).unwrap();
        assert!(request(&home).is_none());
        lock.lock().unwrap();
        assert!(active(&home));
        assert_eq!(request(&home).as_deref(), Some(id));
        acknowledge(&home, id).unwrap();
        drop(lock);
        assert!(!active(&home));
        assert!(request(&home).is_none());
        fs::write(home.join("media/active-container.json"), "{}").unwrap();
        assert!(!active(&home));
        assert_eq!(request(&home).as_deref(), Some("container-recovery-required"));
        fs::remove_file(home.join("media/active-container.json")).unwrap();
        // All paths were created beneath this unique test directory.
        fs::remove_dir_all(home).unwrap();
    }
}
