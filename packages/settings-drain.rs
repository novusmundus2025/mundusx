//! A live file-lock owner requests a settings boundary; stale files are ignored.
use std::{fs::{self, File, OpenOptions}, io, path::{Path, PathBuf}};

pub struct Guard { home: PathBuf, id: String, _lock: File }

pub fn active(home: &Path) -> bool {
    let Ok(file) = OpenOptions::new().read(true).write(true).open(home.join("settings.lock")) else { return false; };
    matches!(file.try_lock(), Err(std::fs::TryLockError::WouldBlock))
}
pub fn request(home: &Path) -> Option<String> {
    if !active(home) { return None; }
    let id = fs::read_to_string(home.join("settings-request.txt")).ok()?;
    (id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit())).then_some(id)
}
pub fn acknowledge(home: &Path, id: &str) -> io::Result<()> {
    fs::write(home.join("settings-ready.txt"), id)
}
pub fn can_acknowledge(active_jobs: usize, media_idle: bool, media_active: bool, pending_completion: bool) -> bool {
    active_jobs == 0 && media_idle && !media_active && !pending_completion
}
impl Guard {
    pub fn acquire(home: &Path) -> io::Result<Self> {
        fs::create_dir_all(home)?;
        let file = OpenOptions::new().create(true).truncate(false).read(true).write(true).open(home.join("settings.lock"))?;
        file.try_lock().map_err(|e| io::Error::other(format!("Another settings session is active: {e}")))?;
        let id = uuid::Uuid::new_v4().simple().to_string();
        fs::write(home.join("settings-request.txt"), &id)?;
        Ok(Self { home: home.into(), id, _lock: file })
    }
    pub fn ready(&self) -> bool {
        fs::read_to_string(self.home.join("settings-ready.txt")).ok().as_deref() == Some(self.id.as_str())
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        let _ = fs::remove_file(self.home.join("settings-request.txt"));
        let _ = fs::remove_file(self.home.join("settings-ready.txt"));
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn waits_for_llm_media_and_completion_delivery_before_settings_boundary() {
        assert!(!can_acknowledge(1, true, false, false));
        assert!(!can_acknowledge(0, false, false, false));
        assert!(!can_acknowledge(0, true, true, false));
        assert!(!can_acknowledge(0, true, false, true));
        assert!(can_acknowledge(0, true, false, false));
    }
    #[test]
    fn requires_live_owner_and_matching_acknowledgment() {
        let home = std::env::temp_dir().join(format!("settings-drain-{}", uuid::Uuid::new_v4()));
        let guard = Guard::acquire(&home).unwrap();
        assert!(active(&home));
        assert!(!guard.ready());
        acknowledge(&home, "old-session").unwrap();
        assert!(!guard.ready());
        acknowledge(&home, &request(&home).unwrap()).unwrap();
        assert!(guard.ready());
        assert!(Guard::acquire(&home).is_err());
        drop(guard);
        assert!(!active(&home));
        fs::write(home.join("settings-request.txt"), "a".repeat(32)).unwrap();
        assert!(request(&home).is_none());
        fs::remove_dir_all(home).unwrap();
    }
}
