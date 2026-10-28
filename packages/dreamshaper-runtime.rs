//! Windows SD1.5 worker. Remote jobs supply text and a seed, never commands or paths.
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
pub const ID: &str = "dreamshaper-8-sd15-512-v1";
pub const MODEL: &str = "Lykon/dreamshaper-8";
pub const MODEL_SHA: &str = "879db523c30d3b9017143d56705015e15a2cb5628762c11d086fed9538abd7fd";
pub const ARCHIVE_SHA: &str = "259ceeb62f1069190d24c85ea1c4549d2d61deb0ef712510dbe98d79ce93a38b";
pub fn root(home: &Path) -> PathBuf {
    home.join("image-models/dreamshaper-8")
}
fn read(path: &Path) -> Result<Value, String> {
    let b = fs::read(path).map_err(|e| e.to_string())?;
    if b.len() > 1024 * 1024 {
        return Err("Image metadata too large".into());
    }
    serde_json::from_slice(&b).map_err(|e| e.to_string())
}
pub fn selected(home: &Path) -> bool {
    cfg!(windows)
        && read(&home.join("image-models/selected.json"))
            .ok()
            .is_some_and(|v| v["profile_id"] == ID)
}
fn digest(path: &Path) -> Result<String, String> {
    let mut f = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut h = Sha256::new();
    let mut b = vec![0u8; 64 * 1024];
    loop {
        let n = f.read(&mut b).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        h.update(&b[..n]);
    }
    Ok(hex::encode(h.finalize()))
}
fn checked_digest(path: &Path, expected: &str) -> Result<(), String> {
    // Heartbeats reuse only same-process checks with unchanged file identity metadata.
    static CACHE: std::sync::OnceLock<
        std::sync::Mutex<std::collections::BTreeMap<PathBuf, (u64, std::time::SystemTime, String)>>,
    > = std::sync::OnceLock::new();
    let m = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !m.is_file() || m.file_type().is_symlink() {
        return Err("Image runtime requires regular files".into());
    }
    let modified = m.modified().map_err(|e| e.to_string())?;
    let mut c = CACHE
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| "Image verification lock")?;
    if c.get(path)
        .is_some_and(|v| v.0 == m.len() && v.1 == modified && v.2 == expected)
    {
        return Ok(());
    }
    if digest(path)? != expected {
        return Err("Image model/runtime checksum changed; verify again".into());
    }
    c.insert(path.into(), (m.len(), modified, expected.into()));
    Ok(())
}
pub fn gpu_mb() -> Option<u64> {
    let exe = PathBuf::from(std::env::var_os("SystemRoot")?).join("System32/nvidia-smi.exe");
    let mut c = Command::new(exe);
    c.args(["--query-gpu=memory.total", "--format=csv,noheader,nounits"]);
    hide(&mut c);
    let o = c.output().ok()?;
    if !o.status.success() {
        return None;
    }
    String::from_utf8(o.stdout)
        .ok()?
        .lines()
        .next()?
        .trim()
        .parse()
        .ok()
}
fn hide(c: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x08000000);
    }
    let _ = c;
}
pub struct ChildGuard {
    #[cfg(windows)]
    handle: windows_sys::Win32::Foundation::HANDLE,
}
#[cfg(windows)]
impl Drop for ChildGuard {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.handle);
        }
    }
}
pub fn own_child(child: &std::process::Child) -> Result<ChildGuard, String> {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::JobObjects::*;
        unsafe {
            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if handle.is_null() {
                return Err("Cannot create image process owner".into());
            }
            let guard = ChildGuard { handle };
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const _,
                std::mem::size_of_val(&limits) as u32,
            ) == 0
                || AssignProcessToJobObject(handle, child.as_raw_handle()) == 0
            {
                return Err("Cannot bind image process lifetime to its worker".into());
            }
            return Ok(guard);
        }
    }
    #[cfg(not(windows))]
    {
        let _ = child;
        Ok(ChildGuard {})
    }
}
pub fn require(home: &Path, cap: u8) -> Result<Value, String> {
    if !selected(home) {
        return Err("Select DreamShaper in OpenGPU first".into());
    }
    if cap == 0 || cap > 100 || gpu_mb().unwrap_or(0) * u64::from(cap) / 100 < 2560 {
        return Err(
            "DreamShaper requires at least 2560 MiB of contributed NVIDIA GPU memory".into(),
        );
    }
    if super::memory::physical_memory_bytes().unwrap_or(0) * u64::from(cap) / 100
        < 8 * super::memory::GIB
    {
        return Err("DreamShaper requires at least 8 GiB system RAM".into());
    }
    let r = root(home);
    let proof = read(&r.join("verified.json"))?;
    if proof["modelSHA256"] != MODEL_SHA || proof["runtimeSHA256"] != ARCHIVE_SHA {
        return Err("Generate a verified DreamShaper test image first".into());
    }
    checked_digest(&r.join("model.safetensors"), MODEL_SHA)?;
    checked_digest(&r.join("runtime.zip"), ARCHIVE_SHA)?;
    let manifest = read(&r.join("runtime-files.json"))?;
    let files = manifest
        .as_object()
        .ok_or("Invalid image runtime manifest")?;
    if !files.contains_key("sd-cli.exe")
        || !files.contains_key("stable-diffusion.dll")
        || !files.contains_key("ggml-vulkan.dll")
    {
        return Err("Incomplete image runtime".into());
    }
    for (name, hash) in files {
        if name.is_empty() || name.contains(['/', '\\', ':']) || name == ".." {
            return Err("Invalid runtime filename".into());
        }
        let expected = hash.as_str().ok_or("Invalid runtime checksum")?;
        checked_digest(&r.join("runtime").join(name), expected)?;
    }
    Ok(json!({"ready":true,"profile_id":ID,"model":MODEL,"cap_percent":cap}))
}
pub fn verification(home: &Path, cap: u8) -> Option<Value> {
    if !selected(home) {
        return None;
    }
    Some(match require(home, cap) {
        Ok(v) => v,
        Err(reason) => json!({"ready":false,"profile_id":ID,"reason":reason}),
    })
}
pub fn generate(home: &Path, cap: u8, prompt: &str, seed: u64) -> Result<Value, String> {
    require(home, cap)?;
    if prompt.trim().is_empty()
        || prompt.len() > 16000
        || prompt.chars().any(|c| c == '\0')
        || seed > 2147483647
    {
        return Err("Invalid image prompt or seed".into());
    }
    let out = home.join("media/artifacts");
    fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    let output = out.join(format!("dreamshaper-{}.png", uuid::Uuid::new_v4()));
    let started = Instant::now();
    let mut cmd = Command::new(root(home).join("runtime/sd-cli.exe"));
    cmd.args(["-m"])
        .arg(root(home).join("model.safetensors"))
        .args([
            "-p",
            prompt,
            "-W",
            "512",
            "-H",
            "512",
            "--steps",
            "20",
            "--cfg-scale",
            "7",
            "--sampling-method",
            "euler",
            "--seed",
            &seed.to_string(),
            "--backend",
            "te=cpu,vae=cpu,diffusion=vulkan0",
            "--offload-to-cpu",
            "--batched-cfg",
            "off",
            "--max-vram",
            &format!(
                "{:.2}",
                gpu_mb().unwrap_or(0) as f64 * f64::from(cap) / 100.0 / 1024.0
            ),
            "-o",
        ])
        .arg(&output)
        .stdin(Stdio::null());
    hide(&mut cmd);
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    let _owner = match own_child(&child) {
        Ok(owner) => owner,
        Err(e) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(e);
        }
    };
    let status = loop {
        if let Some(s) = child.try_wait().map_err(|e| e.to_string())? {
            break s;
        }
        if started.elapsed() > Duration::from_secs(1800) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Image generation timed out".into());
        }
        std::thread::sleep(Duration::from_millis(250));
    };
    if !status.success() {
        return Err("Image runtime failed; see worker diagnostics".into());
    }
    let bytes = fs::read(&output).map_err(|e| e.to_string())?;
    if bytes.len() < 33
        || bytes.len() > 32 * 1024 * 1024
        || bytes[..8] != [137, 80, 78, 71, 13, 10, 26, 10]
        || bytes[16..24] != [0, 0, 2, 0, 0, 0, 2, 0]
    {
        return Err("Image runtime returned an invalid 512x512 PNG".into());
    }
    Ok(
        json!({"artifact":output,"sha256":hex::encode(Sha256::digest(&bytes)),"bytes":bytes.len(),"duration_ms":started.elapsed().as_millis() as u64}),
    )
}
pub fn upload(server: &str, ticket: &Value, result: &Value) -> Result<(), String> {
    let t = ticket.get("upload_ticket").unwrap_or(ticket);
    let id = t["artifact_id"].as_str().ok_or("Missing artifact ID")?;
    if uuid::Uuid::parse_str(id).is_err()
        || t["version"] != 1
        || t["content_type"] != "image/png"
        || t["sha256"] != result["sha256"]
        || t["byte_size"] != result["bytes"]
    {
        return Err("Invalid image upload reservation".into());
    }
    let path = format!("/api/media/artifacts/{id}/content");
    if t["upload_path"] != path {
        return Err("Unscoped upload path".into());
    }
    let token = t["upload_token"]
        .as_str()
        .filter(|s| s.len() == 64 && s.bytes().all(|c| c.is_ascii_hexdigit()))
        .ok_or("Invalid upload token")?;
    let bytes = fs::read(result["artifact"].as_str().ok_or("Missing output")?)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 32 * 1024 * 1024 || hex::encode(Sha256::digest(&bytes)) != t["sha256"] {
        return Err("Upload image changed".into());
    }
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(120))
        .redirects(0)
        .build();
    let response = agent
        .put(&format!("{}{path}", server.trim_end_matches('/')))
        .set("Authorization", &format!("Bearer {token}"))
        .set("Content-Type", "image/png")
        .send_bytes(&bytes)
        .map_err(|_| "Image upload failed; artifact preserved")?;
    let receipt: Value = serde_json::from_reader(response.into_reader().take(16385))
        .map_err(|_| "Invalid image upload receipt")?;
    if receipt["status"] != "ready"
        || receipt["artifact_id"] != id
        || receipt["sha256"] != t["sha256"]
        || receipt["byte_size"] != t["byte_size"]
    {
        return Err("Image upload was not verified".into());
    }
    Ok(())
}
