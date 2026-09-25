//! Embedded media helper used by the CLI and local node worker.
use serde_json::Value;
use std::fs;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub const PROFILE: &str = include_str!("../workers/media/qwen-image-v1.json");
pub const VIDEO_PROFILE: &str = include_str!("../workers/media/wan-video-v1.json");
const HELPER: &str = include_str!("../workers/media/runtime.py");
const UPLOADER: &str = include_str!("../workers/media/artifact_upload.py");

fn write_if_changed(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if fs::read(path).ok().as_deref() == Some(bytes) {
        return Ok(());
    }
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4().simple()));
    fs::write(&temporary, bytes).map_err(|e| e.to_string())?;
    fs::rename(&temporary, path).map_err(|e| e.to_string())
}

pub fn prepare(home: &Path) -> Result<(PathBuf, PathBuf), String> {
    let dir = home.join("media").join("helper");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let helper = dir.join("runtime.py");
    let profile = dir.join("qwen-image-v1.json");
    write_if_changed(&helper, HELPER.as_bytes())?;
    write_if_changed(&dir.join("artifact_upload.py"), UPLOADER.as_bytes())?;
    write_if_changed(&profile, PROFILE.as_bytes())?;
    write_if_changed(&dir.join("wan-video-v1.json"), VIDEO_PROFILE.as_bytes())?;
    Ok((helper, profile))
}

pub fn run(
    home: &Path,
    cap: u8,
    endpoint: Option<&str>,
    action: &str,
    extra: &[String],
) -> Result<(), String> {
    run_profile(home, cap, endpoint, action, false, extra)
}

pub fn run_profile(
    home: &Path,
    cap: u8,
    endpoint: Option<&str>,
    action: &str,
    video: bool,
    extra: &[String],
) -> Result<(), String> {
    let (helper, profile) = prepare(home)?;
    let profile = if video {
        helper.with_file_name("wan-video-v1.json")
    } else {
        profile
    };
    let python = ["python3", "python"]
        .into_iter()
        .find(|program| {
            Command::new(program)
                .args([
                    "-c",
                    "import sys; sys.exit(0 if sys.version_info >= (3, 12) else 1)",
                ])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|status| status.success())
        })
        .ok_or("Media setup needs Python 3.12 or newer on PATH")?;
    let mut command = Command::new(python);
    command
        .arg("-u")
        .arg(helper)
        .arg(action)
        .arg("--home")
        .arg(home)
        .arg("--profile")
        .arg(profile)
        .arg("--cap-percent")
        .arg(cap.to_string())
        .args(extra);
    if let Some(endpoint) = endpoint {
        command.arg("--endpoint").arg(endpoint);
    }
    if std::io::stdout().is_terminal() && std::io::stderr().is_terminal() {
        command.env("OPENGPU_MEDIA_HUMAN_PROGRESS", "1");
    }
    let status = command
        .stdin(Stdio::null())
        .status()
        .map_err(|e| format!("Cannot start media helper: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("Media operation did not finish; see its diagnostic above".into())
    }
}

pub fn verification(home: &Path, endpoint: Option<&str>, cap: u8) -> Option<Value> {
    profile_verification(home, endpoint, cap, false)
}

pub fn video_verification(home: &Path, endpoint: Option<&str>, cap: u8) -> Option<Value> {
    profile_verification(home, endpoint, cap, true)
}

fn profile_verification(
    home: &Path,
    endpoint: Option<&str>,
    cap: u8,
    video: bool,
) -> Option<Value> {
    use sha2::{Digest, Sha256};
    let mut value: Value = serde_json::from_slice(
        &fs::read(home.join(if video {
            "media/verified-video.json"
        } else {
            "media/verified.json"
        }))
        .ok()?,
    )
    .ok()?;
    let profile: Value = serde_json::from_str(if video { VIDEO_PROFILE } else { PROFILE }).ok()?;
    let hash = hex::encode(Sha256::digest(serde_json::to_vec(&profile).ok()?));
    let endpoint_matches = value.get("endpoint").and_then(Value::as_str) == endpoint;
    let matches = value["profile_hash"].as_str() == Some(&hash)
        && value["cap_percent"].as_u64() == Some(u64::from(cap))
        && endpoint_matches
        && !home.join("media/active-container.json").exists();
    if !matches {
        value["ready"] = Value::Bool(false);
        value["reason"] = "Configuration or bundled profile changed; verify again".into();
    }
    Some(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn profile_has_pinned_sources_and_only_image_generation() {
        let profile: Value = serde_json::from_str(PROFILE).unwrap();
        assert_eq!(profile["operation"], "text_to_image");
        assert_eq!(profile["comfy_revision"].as_str().unwrap().len(), 40);
        assert_eq!(profile["models_revision"].as_str().unwrap().len(), 40);
        for file in profile["files"].as_array().unwrap() {
            assert_eq!(file["sha256"].as_str().unwrap().len(), 64);
        }
    }
    #[test]
    fn video_certificate_does_not_replace_or_reuse_image_readiness() {
        use sha2::{Digest, Sha256};
        let home = std::env::temp_dir().join(format!("opengpu-video-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(home.join("media")).unwrap();
        let profile: Value = serde_json::from_str(VIDEO_PROFILE).unwrap();
        let certificate = serde_json::json!({"ready": true, "profile_hash": hex::encode(Sha256::digest(serde_json::to_vec(&profile).unwrap())), "cap_percent": 65, "endpoint": null});
        fs::write(
            home.join("media/verified-video.json"),
            serde_json::to_vec(&certificate).unwrap(),
        )
        .unwrap();
        assert!(verification(&home, None, 65).is_none());
        assert_eq!(video_verification(&home, None, 65).unwrap()["ready"], true);
        assert_eq!(video_verification(&home, None, 70).unwrap()["ready"], false);
        fs::write(home.join("media/active-container.json"), "{}").unwrap();
        assert_eq!(video_verification(&home, None, 65).unwrap()["ready"], false);
        fs::remove_dir_all(&home).unwrap();
    }
}
