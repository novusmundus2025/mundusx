use crate::{
    config::{self, Config},
    identity::{self, DeviceIdentity},
    media_runtime,
};
use serde_json::{json, Value};
use std::io::Read;
use std::{
    fs,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
fn request(
    server: &str,
    identity: &DeviceIdentity,
    node: &str,
    action: &str,
    mut body: Value,
) -> Result<Value, String> {
    let url = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(120))
        .redirects(0)
        .build();
    body["_nonce"] = json!(uuid::Uuid::new_v4().to_string());
    let text = serde_json::to_string(&body).map_err(|e| e.to_string())?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_secs()
        .to_string();
    let path = format!("/api/media/worker/{action}");
    let signature = identity
        .sign_hex(&format!("POST\n{path}\n{stamp}\n{text}"))
        .map_err(|e| e.to_string())?;
    let response = url
        .post(&format!("{}{path}", server.trim_end_matches('/')))
        .set("Content-Type", "application/json")
        .set("X-MundusX-Node-Id", node)
        .set("X-MundusX-Timestamp", &stamp)
        .set("X-MundusX-Signature", &signature)
        .send_string(&text)
        .map_err(|e| match e {
            ureq::Error::Status(code, _) => format!("Media server returned HTTP {code}"),
            _ => "Media server connection failed".into(),
        })?;
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(46 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Cannot read media server response")?;
    if bytes.len() > 46 * 1024 * 1024 {
        return Err("Media server response too large".into());
    }
    serde_json::from_slice(&bytes).map_err(|_| "Invalid media server response".into())
}
fn private_json(path: &std::path::Path, value: &Value) -> Result<(), String> {
    use std::io::Write;
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|e| e.to_string())?;
    file.write_all(serde_json::to_string(value).unwrap().as_bytes())
        .map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())
}
fn execution_profile(job: &Value) -> Result<(bool, u64, String), String> {
    let i2v = job["quote"]["operation"] == "image_to_video";
    let video = i2v || job["quote"]["operation"] == "text_to_video";
    let (seconds, name) = if video {
        let frames = job["quote"]["frames"]
            .as_u64()
            .ok_or("Missing video frames")?;
        let profile: Value = serde_json::from_str(if i2v {
            media_runtime::I2V_FAST_PROFILE
        } else {
            media_runtime::VIDEO_PROFILE
        })
        .map_err(|e| e.to_string())?;
        let fps = profile["fps"].as_u64().ok_or("Missing video FPS")?;
        let base_frames = profile["frames"].as_u64().ok_or("Missing base frames")?;
        let seconds = (1..=10)
            .find(|s| s * fps + 1 == frames)
            .ok_or("Unsupported video preset")?;
        let expected = profile["id"]
            .as_str()
            .ok_or("Missing bundled video profile")?
            .replace(&format!("-{base_frames}f-"), &format!("-{frames}f-"));
        if job["profile_id"].as_str() != Some(expected.as_str())
            || job["quote"]["fps"].as_u64() != Some(fps)
        {
            return Err("Job model does not match the installed video workflow".into());
        }
        let suffix = if i2v { "-i2v-fast" } else { "-video" };
        let name = if frames == base_frames {
            format!("verified{suffix}.json")
        } else {
            format!("verified{suffix}-{frames}f.json")
        };
        (seconds, name)
    } else {
        let profile: Value =
            serde_json::from_str(media_runtime::PROFILE).map_err(|e| e.to_string())?;
        if job["quote"]["operation"] != "text_to_image" || job["profile_id"] != profile["id"] {
            return Err("Job does not match the installed image workflow".into());
        }
        (2, "verified.json".to_string())
    };
    Ok((video, seconds, name))
}

// Only delete exact worker-owned files; never traverse arbitrary output paths.
fn cleanup_completed_media(home: &std::path::Path, cleanup: &Value) -> Result<(), String> {
    let root = home.join("media");
    let mut files = Vec::new();
    for (key, directory) in [
        ("artifact_name", "artifacts"),
        ("comfy_output_filename", "outputs/opengpu"),
    ] {
        if let Some(name) = cleanup[key].as_str() {
            if name.is_empty()
                || name.contains('/')
                || name.contains('\\')
                || name.contains(':')
                || name == "."
                || name == ".."
            {
                return Err("Invalid media cleanup filename".into());
            }
            let path = root.join(directory).join(name);
            if path.exists() {
                let resolved = path.canonicalize().map_err(|e| e.to_string())?;
                let parent = root
                    .join(directory)
                    .canonicalize()
                    .map_err(|e| e.to_string())?;
                let owned_root = root.canonicalize().map_err(|e| e.to_string())?;
                if !parent.starts_with(&owned_root) || resolved.parent() != Some(parent.as_path()) {
                    return Err("Media cleanup path escaped its output directory".into());
                }
                if let Some(expected) = cleanup["sha256"].as_str() {
                    use sha2::{Digest, Sha256};
                    let bytes = fs::read(&path).map_err(|e| e.to_string())?;
                    if hex::encode(Sha256::digest(&bytes)) != expected {
                        return Err("Completed media copy changed; cleanup refused".into());
                    }
                }
            }
            files.push(path);
        }
    }
    for path in files {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(())
}

fn execute(server: &str, cfg: &Config, id: &DeviceIdentity, job: &Value) -> Result<(), String> {
    execute_with(
        cfg,
        job,
        &config::config_dir(),
        server,
        |action, video, extra| {
            media_runtime::run_profile(
                &config::config_dir(),
                cfg.contribution_percent,
                if action == "upload" {
                    None
                } else {
                    cfg.contribution.comfyui_url.as_deref()
                },
                action,
                video,
                extra,
            )
        },
        |action, body| request(server, id, &cfg.device_id, action, body),
    )
}

fn execute_with(
    cfg: &Config,
    job: &Value,
    home: &std::path::Path,
    server: &str,
    mut run: impl FnMut(&str, bool, &[String]) -> Result<(), String>,
    mut call: impl FnMut(&str, Value) -> Result<Value, String>,
) -> Result<(), String> {
    let (video, seconds, name) = execution_profile(job)?;
    let i2v = job["quote"]["operation"] == "image_to_video";
    let operation = if i2v {
        crate::contribution_contract::Operation::ImageToVideo
    } else if video {
        crate::contribution_contract::Operation::TextToVideo
    } else {
        crate::contribution_contract::Operation::TextToImage
    };
    if !cfg.contribution.operations.contains(&operation) {
        return Err("Claimed media workload is not selected by this contributor".into());
    }
    let mut generation_args = vec![
        "--seconds".into(),
        seconds.to_string(),
        "--prompt".into(),
        job["prompt"].as_str().ok_or("Missing prompt")?.into(),
        "--seed".into(),
        job["seed"].as_u64().unwrap_or(42).to_string(),
    ];
    let reference_path = home
        .join("media")
        .join(format!("reference-{}.png", uuid::Uuid::new_v4()));
    if i2v {
        use base64::Engine;
        use sha2::{Digest, Sha256};
        let reference = call(
            "input",
            json!({"job_id":job["job_id"], "lease_token":job["lease_token"]}),
        )?;
        let encoded = reference["data"]
            .as_str()
            .ok_or("Missing reference image")?;
        if encoded.len() > 45 * 1024 * 1024 {
            return Err("Reference image too large".into());
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|_| "Invalid reference image")?;
        if hex::encode(Sha256::digest(&bytes)) != reference["sha256"].as_str().unwrap_or("") {
            return Err("Reference image checksum mismatch".into());
        }
        fs::write(&reference_path, bytes).map_err(|e| e.to_string())?;
        generation_args.extend([
            "--image-to-video".into(),
            "--fast".into(),
            "--input-image".into(),
            reference_path.to_string_lossy().into_owned(),
        ]);
    }
    let generated = run("generate", video, &generation_args);
    if i2v {
        let _ = fs::remove_file(&reference_path);
    }
    generated?;
    let result: Value = serde_json::from_slice(
        &fs::read(home.join("media").join(name)).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let lease = json!({"job_id":job["job_id"],"lease_token":job["lease_token"]});
    let mut reserve = lease.clone();
    reserve["metadata"] = json!({"sha256":result["sha256"],"byte_size":result["bytes"]});
    let ticket = call("reserve", reserve)?;
    let ticket_file = home
        .join("media")
        .join(format!("upload-{}.json", uuid::Uuid::new_v4()));
    private_json(&ticket_file, &ticket)?;
    let mut upload_args = vec![
        "--file".into(),
        result["artifact"]
            .as_str()
            .ok_or("Missing artifact")?
            .into(),
        "--ticket".into(),
        ticket_file.to_string_lossy().into_owned(),
        "--server".into(),
        server.into(),
    ];
    if !video && cfg.contribution.comfyui_url.is_none() {
        if let Some(name) = result["source_filename"].as_str() {
            upload_args.extend(["--managed-output".into(), name.into()]);
        }
    }
    let upload = run("upload", video, &upload_args);
    let _ = fs::remove_file(&ticket_file);
    upload?;
    let mut complete = lease;
    complete["generation_ms"] = result["duration_ms"].clone();
    let artifact = std::path::Path::new(result["artifact"].as_str().ok_or("Missing artifact")?);
    if artifact.parent() != Some(home.join("media/artifacts").as_path()) {
        return Err("Generated artifact is outside the worker output directory".into());
    }
    complete["_local_cleanup"] =
        json!({"artifact_name": artifact.file_name().and_then(|name| name.to_str()), "sha256": result["sha256"]});
    if cfg.contribution.comfyui_url.is_none() {
        complete["_local_cleanup"]["comfy_output_filename"] = result["source_filename"].clone();
    }
    let pending = home.join("media/pending-completion.json");
    private_json(&pending, &complete)?;
    for attempt in 0..4 {
        let mut body = complete.clone();
        body.as_object_mut().unwrap().remove("_local_cleanup");
        match call("complete", body) {
            Ok(_) => {
                cleanup_completed_media(&home, &complete["_local_cleanup"])?;
                let _ = fs::remove_file(&pending);
                println!("Media job completed and credits settled");
                return Ok(());
            }
            Err(e) => {
                if attempt == 3 {
                    return Err(e);
                }
                thread::sleep(Duration::from_secs(10));
            }
        }
    }
    unreachable!()
}
pub fn serve(server: String, once: bool) -> Result<(), String> {
    if !(server.starts_with("https://") || server.starts_with("http://127.0.0.1:"))
        || server.contains('@')
        || server.contains('?')
        || server.contains('#')
    {
        return Err("Use an HTTPS media-server origin".into());
    }
    let id = identity::load_identity()
        .map_err(|e| e.to_string())?
        .ok_or("Run opengpu install first")?;
    let home = config::config_dir();
    fs::create_dir_all(home.join("media")).map_err(|e| e.to_string())?;
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(home.join("media/worker.lock"))
        .map_err(|e| e.to_string())?;
    lock.try_lock()
        .map_err(|_| "A media worker is already running")?;
    println!("Media contributor listening for queued jobs");
    loop {
        let cfg = config::load_config()
            .map_err(|e| e.to_string())?
            .ok_or("Missing contributor configuration")?;
        if cfg.paused
            || (std::env::var("OPENGPU_MEDIA_MANAGED").as_deref() == Ok("true") && !cfg.connected)
        {
            return Ok(());
        }
        let profiles: Vec<_> = media_runtime::verified_profiles(
            &home,
            cfg.contribution.comfyui_url.as_deref(),
            cfg.contribution_percent,
        )
        .into_iter()
        .filter(|profile| {
            let op = if profile.starts_with("qwen-") {
                crate::contribution_contract::Operation::TextToImage
            } else if profile.starts_with("wan22-i2v-") {
                crate::contribution_contract::Operation::ImageToVideo
            } else {
                crate::contribution_contract::Operation::TextToVideo
            };
            cfg.contribution.operations.contains(&op)
        })
        .collect();
        if profiles.is_empty() {
            return Err(
                "Select image/video workloads and verify at least one profile before serving media"
                    .into(),
            );
        }
        let pending = home.join("media/pending-completion.json");
        if pending.exists() {
            let body: Value =
                serde_json::from_slice(&fs::read(&pending).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            let mut completion = body.clone();
            completion
                .as_object_mut()
                .ok_or("Invalid pending completion")?
                .remove("_local_cleanup");
            request(&server, &id, &cfg.device_id, "complete", completion)?;
            cleanup_completed_media(&home, &body["_local_cleanup"])?;
            fs::remove_file(&pending).map_err(|e| e.to_string())?;
        }
        let claim = match request(
            &server,
            &id,
            &cfg.device_id,
            "claim",
            json!({"profiles": profiles}),
        ) {
            Ok(value) => value,
            Err(error) => {
                if once {
                    return Err(error);
                }
                eprintln!("Media queue: {error}; retrying in 30 seconds");
                thread::sleep(Duration::from_secs(30));
                continue;
            }
        };
        if !claim["job"].is_null() {
            let job = &claim["job"];
            let done = Arc::new(AtomicBool::new(false));
            let stopped = done.clone();
            let server2 = server.clone();
            let identity2 = id.clone();
            let node = cfg.device_id.clone();
            let lease = json!({"job_id":job["job_id"],"lease_token":job["lease_token"]});
            let heartbeat = lease.clone();
            let thread = thread::spawn(move || {
                let mut seconds = 0;
                while !stopped.load(Ordering::Relaxed) {
                    thread::sleep(Duration::from_secs(1));
                    seconds += 1;
                    if seconds % 30 == 0 {
                        if let Err(e) =
                            request(&server2, &identity2, &node, "heartbeat", heartbeat.clone())
                        {
                            eprintln!("Media lease heartbeat: {e}");
                        }
                    }
                }
            });
            let result = execute(&server, &cfg, &id, job);
            done.store(true, Ordering::Relaxed);
            let _ = thread.join();
            if let Err(e) = result {
                eprintln!("Media job failed: {e}");
                if !pending.exists() {
                    let _ = request(&server, &id, &cfg.device_id, "fail", lease);
                } else {
                    return Err(e);
                }
            }
        }
        if once {
            return Ok(());
        }
        thread::sleep(Duration::from_secs(10));
    }
}

fn bundled_video_profiles() -> Result<Vec<String>, String> {
    let profile: Value =
        serde_json::from_str(media_runtime::VIDEO_PROFILE).map_err(|e| e.to_string())?;
    let id = profile["id"]
        .as_str()
        .ok_or("Missing bundled video profile")?;
    let fps = profile["fps"].as_u64().ok_or("Missing video FPS")?;
    let base_frames = profile["frames"].as_u64().ok_or("Missing base frames")?;
    Ok((1..=10)
        .map(|seconds| {
            id.replace(
                &format!("-{base_frames}f-"),
                &format!("-{}f-", seconds * fps + 1),
            )
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn video_copies_are_kept_until_completion_acknowledgement() {
        use sha2::{Digest, Sha256};
        let home = std::env::temp_dir().join(format!("video-cleanup-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(home.join("media/artifacts")).unwrap();
        fs::create_dir_all(home.join("media/outputs/opengpu")).unwrap();
        let artifact = home.join("media/artifacts/test.mp4");
        let source = home.join("media/outputs/opengpu/test.mp4");
        let mut cfg = Config::default();
        cfg.contribution.operations = vec![crate::contribution_contract::Operation::TextToVideo];
        let profile: Value = serde_json::from_str(media_runtime::VIDEO_PROFILE).unwrap();
        let job = json!({"job_id":"test", "lease_token":"lease", "profile_id":profile["id"],
            "quote":{"operation":"text_to_video", "frames":profile["frames"], "fps":profile["fps"]}, "prompt":"mountains"});
        execute_with(&cfg, &job, &home, "https://chat.example", |action, _, _| {
            if action == "generate" {
                fs::write(&artifact, b"video").unwrap();
                fs::write(&source, b"video").unwrap();
                private_json(&home.join("media/verified-video.json"), &json!({
                    "artifact":artifact, "source_filename":"test.mp4", "sha256":hex::encode(Sha256::digest(b"video")), "bytes":5, "duration_ms":100}))?;
            }
            Ok(())
        }, |action, body| {
            assert!(artifact.exists() && source.exists());
            assert!(body.get("_local_cleanup").is_none());
            Ok(if action == "reserve" { json!({"upload_ticket":{}}) } else { json!({"status":"completed"}) })
        }).unwrap();
        assert!(!artifact.exists() && !source.exists());
        assert!(!home.join("media/pending-completion.json").exists());
        fs::remove_file(home.join("media/verified-video.json")).unwrap();
        for directory in ["media/artifacts", "media/outputs/opengpu", "media/outputs", "media", ""] {
            fs::remove_dir(home.join(directory)).unwrap();
        }
    }
    #[test]
    fn completed_media_cleanup_is_scoped_and_restart_safe() {
        let home = std::env::temp_dir().join(format!("media-cleanup-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(home.join("media/artifacts")).unwrap();
        fs::create_dir_all(home.join("media/outputs/opengpu")).unwrap();
        let artifact = home.join("media/artifacts/generated.png");
        let source = home.join("media/outputs/opengpu/generated.png");
        let unrelated = home.join("media/artifacts/keep.png");
        for path in [&artifact, &source, &unrelated] {
            fs::write(path, b"image").unwrap();
        }
        assert!(cleanup_completed_media(&home, &json!({"artifact_name":"../keep.png"})).is_err());
        assert!(artifact.exists());
        let cleanup =
            json!({"artifact_name":"generated.png", "comfy_output_filename":"generated.png"});
        cleanup_completed_media(&home, &cleanup).unwrap();
        cleanup_completed_media(&home, &cleanup).unwrap();
        assert!(!artifact.exists());
        assert!(!source.exists());
        assert!(unrelated.exists());
        fs::remove_file(unrelated).unwrap();
        fs::remove_dir(home.join("media/artifacts")).unwrap();
        fs::remove_dir(home.join("media/outputs/opengpu")).unwrap();
        fs::remove_dir(home.join("media/outputs")).unwrap();
        fs::remove_dir(home.join("media")).unwrap();
        fs::remove_dir(home).unwrap();
    }
    #[test]
    fn unselected_image_job_never_generates_or_uploads() {
        let cfg = Config::default();
        let profile: Value = serde_json::from_str(media_runtime::PROFILE).unwrap();
        let job = json!({"profile_id":profile["id"],"quote":{"operation":"text_to_image"}});
        assert!(execute_with(
            &cfg,
            &job,
            std::path::Path::new("."),
            "https://chat.example",
            |_, _, _| panic!("unselected workload must not generate"),
            |_, _| panic!("unselected workload must not upload")
        )
        .unwrap_err()
        .contains("not selected"));
    }

    #[test]
    fn image_job_generates_uploads_then_completes_and_never_completes_failed_upload() {
        for fail_upload in [false, true] {
            let home = std::env::temp_dir().join(format!("image-job-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(home.join("media")).unwrap();
            let mut cfg = Config::default();
            cfg.contribution.operations =
                vec![crate::contribution_contract::Operation::TextToImage];
            let profile: Value = serde_json::from_str(media_runtime::PROFILE).unwrap();
            let job = json!({"job_id":"test-job", "lease_token":"test-lease", "profile_id":profile["id"],
                "quote":{"operation":"text_to_image"}, "prompt":"a blue bird", "seed":17});
            let events = std::cell::RefCell::new(Vec::new());
            let result = execute_with(
                &cfg,
                &job,
                &home,
                "https://chat.example",
                |action, video, args| {
                    assert!(!video);
                    events.borrow_mut().push(action.to_string());
                    if action == "generate" {
                        assert!(args.windows(2).any(|v| v == ["--prompt", "a blue bird"]));
                        private_json(
                            &home.join("media/verified.json"),
                            &json!({
                            "artifact":home.join("media/artifacts/image.png"),"sha256":"digest","bytes":123,"duration_ms":456,
                            "source_filename":"0123456789abcdef0123456789abcdef_00001_.png"}),
                        )?;
                    } else {
                        assert_eq!(action, "upload");
                        assert!(args.windows(2).any(|v| v
                            == [
                                "--managed-output",
                                "0123456789abcdef0123456789abcdef_00001_.png"
                            ]));
                        let ticket = std::path::Path::new(
                            &args[args.iter().position(|v| v == "--ticket").unwrap() + 1],
                        );
                        assert!(ticket.exists());
                        if fail_upload {
                            return Err("Upload failed".into());
                        }
                    }
                    Ok(())
                },
                |action, body| {
                    events.borrow_mut().push(action.to_string());
                    assert_eq!(body["job_id"], "test-job");
                    assert_eq!(body["lease_token"], "test-lease");
                    if action == "reserve" {
                        assert_eq!(body["metadata"]["byte_size"], 123);
                        Ok(json!({"upload_ticket":{"artifact_id":"test-artifact"}}))
                    } else {
                        assert_eq!(action, "complete");
                        assert_eq!(body["generation_ms"], 456);
                        Ok(json!({"status":"completed"}))
                    }
                },
            );
            assert_eq!(result.is_err(), fail_upload);
            assert_eq!(
                *events.borrow(),
                if fail_upload {
                    vec!["generate", "reserve", "upload"]
                } else {
                    vec!["generate", "reserve", "upload", "complete"]
                }
            );
            assert!(!home.join("media/pending-completion.json").exists());
            assert!(!fs::read_dir(home.join("media")).unwrap().any(|p| p
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("upload-")));
            fs::remove_dir_all(home).unwrap();
        }
    }

    #[test]
    fn chooses_image_and_supported_video_profiles_and_rejects_mismatch() {
        let image: Value = serde_json::from_str(media_runtime::PROFILE).unwrap();
        let image_job = json!({"profile_id":image["id"],"quote":{"operation":"text_to_image"}});
        assert_eq!(
            execution_profile(&image_job).unwrap(),
            (false, 2, "verified.json".into())
        );
        let profiles = bundled_video_profiles().unwrap();
        assert_eq!(profiles.len(), 10);
        for seconds in 1..=10_u64 {
            let id = bundled_video_profiles().unwrap()[(seconds - 1) as usize].clone();
            assert!(profiles.contains(&id));
            let mut job = json!({"profile_id":id,"quote":{"operation":"text_to_video","fps":16,"frames":seconds*16+1}});
            let result = execution_profile(&job).unwrap();
            assert!(result.0);
            assert_eq!(result.1, seconds);
            job["quote"]["fps"] = json!(24);
            assert!(execution_profile(&job).is_err());
        }
        assert!(execution_profile(
            &json!({"profile_id":"unknown","quote":{"operation":"text_to_image"}})
        )
        .is_err());
        assert!(execution_profile(
            &json!({"profile_id":image["id"],"quote":{"operation":"image_edit"}})
        )
        .is_err());
    }
    #[test]
    fn lightning_i2v_uses_matching_duration_and_cleans_reference_after_failure() {
        use base64::Engine;
        use sha2::{Digest, Sha256};
        let home = std::env::temp_dir().join(format!("i2v-job-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(home.join("media")).unwrap();
        let mut cfg = Config::default();
        cfg.contribution.operations = vec![crate::contribution_contract::Operation::ImageToVideo];
        let job = json!({"job_id":"test", "lease_token":"lease", "profile_id":"wan22-i2v-lightning-14b-480p-161f-v3",
            "quote":{"operation":"image_to_video", "frames":161, "fps":16}, "prompt":"move"});
        assert_eq!(
            execution_profile(&job).unwrap(),
            (true, 10, "verified-i2v-fast-161f.json".into())
        );
        let bytes = b"reference fixture";
        let result = execute_with(
            &cfg,
            &job,
            &home,
            "https://chat.example",
            |action, video, args| {
                assert_eq!(action, "generate");
                assert!(video);
                assert!(args.iter().any(|arg| arg == "--fast"));
                assert!(args.iter().any(|arg| arg == "--image-to-video"));
                let path = &args[args.iter().position(|arg| arg == "--input-image").unwrap() + 1];
                assert_eq!(fs::read(path).unwrap(), bytes);
                Err("simulated generation failure".into())
            },
            |action, body| {
                assert_eq!(action, "input");
                assert_eq!(body["lease_token"], "lease");
                Ok(
                    json!({"data":base64::engine::general_purpose::STANDARD.encode(bytes), "sha256":hex::encode(Sha256::digest(bytes))}),
                )
            },
        );
        assert!(result.is_err());
        assert_eq!(fs::read_dir(home.join("media")).unwrap().count(), 0);
        fs::remove_dir_all(home).unwrap();
    }
}
