//! Local GPU handoff: the helper holds media.lock while the agent drains LLM work.
use std::{fs, path::Path};

pub fn request(home: &Path) -> Option<String> {
    let media = home.join("media");
    // A crashed helper may leave its container using the GPU. Fail closed until
    // the helper confirms that its owned container has stopped.
    if media.join("active-container.json").exists() {
        return Some("container-recovery-required".to_string());
    }
    let value: serde_json::Value = serde_json::from_slice(&fs::read(media.join("drain-request.json")).ok()?).ok()?;
    let id = value["id"].as_str()?;
    if id.len() != 32 || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) { return None; }
    let file = fs::OpenOptions::new().read(true).write(true).open(media.join("media.lock")).ok()?;
    // A dead helper releases its OS lock even when it cannot remove its marker.
    if file.try_lock().is_ok() { return None; }
    Some(id.to_string())
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
    fn stale_request_without_live_helper_is_ignored() {
        let home = std::env::temp_dir().join(format!("opengpu-drain-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(home.join("media")).unwrap();
        let id = "0123456789abcdef0123456789abcdef";
        fs::write(home.join("media/drain-request.json"), serde_json::json!({"id":id}).to_string()).unwrap();
        let lock = fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(home.join("media/media.lock")).unwrap();
        assert!(request(&home).is_none());
        lock.lock().unwrap();
        assert_eq!(request(&home).as_deref(), Some(id));
        acknowledge(&home, id).unwrap();
        drop(lock);
        assert!(request(&home).is_none());
        fs::write(home.join("media/active-container.json"), "{}").unwrap();
        assert_eq!(request(&home).as_deref(), Some("container-recovery-required"));
        fs::remove_file(home.join("media/active-container.json")).unwrap();
        // All paths were created beneath this unique test directory.
        fs::remove_dir_all(home).unwrap();
    }
}
