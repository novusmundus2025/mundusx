use crate::{
    config::{self, Config},
    identity::{self, DeviceIdentity},
    media_runtime,
};
use serde_json::{json, Value};
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
    response
        .into_json()
        .map_err(|_| "Invalid media server response".into())
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
    let video = job["quote"]["operation"] == "text_to_video";
    let (seconds, name) = if video {
        let frames = job["quote"]["frames"]
            .as_u64()
            .ok_or("Missing video frames")?;
        let profile: Value =
            serde_json::from_str(media_runtime::VIDEO_PROFILE).map_err(|e| e.to_string())?;
        let fps = profile["fps"].as_u64().ok_or("Missing video FPS")?;
        let base_frames = profile["frames"].as_u64().ok_or("Missing base frames")?;
        let seconds = [2, 5, 10]
            .into_iter()
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
        let name = if frames == base_frames {
            "verified-video.json".to_string()
        } else {
            format!("verified-video-{frames}f.json")
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
    let operation = if video {
        crate::contribution_contract::Operation::TextToVideo
    } else {
        crate::contribution_contract::Operation::TextToImage
    };
    if !cfg.contribution.operations.contains(&operation) {
        return Err("Claimed media workload is not selected by this contributor".into());
    }
    run(
        "generate",
        video,
        &[
            "--seconds".into(),
            seconds.to_string(),
            "--prompt".into(),
            job["prompt"].as_str().ok_or("Missing prompt")?.into(),
            "--seed".into(),
            job["seed"].as_u64().unwrap_or(42).to_string(),
        ],
    )?;
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
    let upload = run(
        "upload",
        video,
        &[
            "--file".into(),
            result["artifact"]
                .as_str()
                .ok_or("Missing artifact")?
                .into(),
            "--ticket".into(),
            ticket_file.to_string_lossy().into_owned(),
            "--server".into(),
            server.into(),
        ],
    );
    let _ = fs::remove_file(&ticket_file);
    upload?;
    let mut complete = lease;
    complete["generation_ms"] = result["duration_ms"].clone();
    let pending = home.join("media/pending-completion.json");
    private_json(&pending, &complete)?;
    for attempt in 0..4 {
        match call("complete", complete.clone()) {
            Ok(_) => {
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
        let mut profiles = Vec::new();
        for video in [false, true] {
            let operation = if video {
                crate::contribution_contract::Operation::TextToVideo
            } else {
                crate::contribution_contract::Operation::TextToImage
            };
            if !cfg.contribution.operations.contains(&operation) {
                continue;
            }
            if !media_runtime::memory::MediaBudget::detect(cfg.contribution_percent)
                .allows(operation)
            {
                continue;
            }
            let verified = if video {
                media_runtime::video_verification(
                    &home,
                    cfg.contribution.comfyui_url.as_deref(),
                    cfg.contribution_percent,
                )
            } else {
                media_runtime::verification(
                    &home,
                    cfg.contribution.comfyui_url.as_deref(),
                    cfg.contribution_percent,
                )
            };
            if verified.is_some_and(|value| value["ready"] == true) {
                if video {
                    profiles.extend(bundled_video_profiles()?);
                } else {
                    let profile: Value =
                        serde_json::from_str(media_runtime::PROFILE).map_err(|e| e.to_string())?;
                    profiles.push(
                        profile["id"]
                            .as_str()
                            .ok_or("Missing image profile")?
                            .to_string(),
                    );
                }
            }
        }
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
            request(&server, &id, &cfg.device_id, "complete", body)?;
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
    Ok([2, 5, 10]
        .into_iter()
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
                            "artifact":home.join("image.png"),"sha256":"digest","bytes":123,"duration_ms":456}),
                        )?;
                    } else {
                        assert_eq!(action, "upload");
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
        assert_eq!(profiles.len(), 3);
        for seconds in [2, 5, 10_u64] {
            let id = bundled_video_profiles().unwrap()
                [[2, 5, 10].iter().position(|s| *s == seconds).unwrap()]
            .clone();
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
}
