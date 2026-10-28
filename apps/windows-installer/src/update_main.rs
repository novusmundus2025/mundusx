#![cfg_attr(windows, windows_subsystem = "windows")]

const UPDATE_SCRIPT: &str = include_str!("../../../install.ps1");

fn update_arguments(script: &std::path::Path) -> Vec<String> {
    let mut arguments = vec![
        "-NoProfile".into(),
        "-ExecutionPolicy".into(),
        "Bypass".into(),
        "-File".into(),
        script.display().to_string(),
        "-SkipModelRuntime".into(),
        "-SkipContributorSetup".into(),
        "-SkipChatConnect".into(),
        "-AgentMode".into(),
        "none".into(),
        "-SetupMode".into(),
        "developer".into(),
    ];
    if let Ok(base) = std::env::var("MUNDUSX_RELEASE_BASE_URL") {
        if !base.trim().is_empty() {
            arguments.extend(["-ReleaseBaseUrl".into(), base.trim().into()]);
        }
    }
    arguments
}

fn run_update() -> Result<(), String> {
    let directory = std::env::temp_dir().join(format!("mundusx-update-{}", std::process::id()));
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("Could not prepare the update: {error}"))?;
    let script = directory.join("update.ps1");
    std::fs::write(&script, UPDATE_SCRIPT)
        .map_err(|error| format!("Could not prepare the update: {error}"))?;
    let result = std::process::Command::new("powershell.exe")
        .args(update_arguments(&script))
        .status()
        .map_err(|error| format!("Could not start the update: {error}"))?;
    let _ = std::fs::remove_dir_all(&directory);
    result.success().then_some(()).ok_or_else(|| {
        format!(
            "MundusX update failed with exit code {}. See .opengpu\\logs\\installer.log for details.",
            result.code().unwrap_or(-1)
        )
    })
}

#[cfg(windows)]
fn message(title: &str, body: &str, error: bool) {
    use std::{ffi::OsStr, os::windows::ffi::OsStrExt, ptr};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, MB_ICONERROR, MB_ICONINFORMATION, MB_OK,
    };
    let title: Vec<u16> = OsStr::new(title).encode_wide().chain(Some(0)).collect();
    let body: Vec<u16> = OsStr::new(body).encode_wide().chain(Some(0)).collect();
    unsafe {
        MessageBoxW(
            ptr::null_mut(),
            body.as_ptr(),
            title.as_ptr(),
            MB_OK
                | if error {
                    MB_ICONERROR
                } else {
                    MB_ICONINFORMATION
                },
        );
    }
}

#[cfg(windows)]
fn main() {
    match run_update() {
        Ok(()) => message(
            "MundusX updated",
            "MundusX was updated from the verified release feed. Return to Chat and refresh the page.",
            false,
        ),
        Err(error) => message("MundusX update failed", &error, true),
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("MundusX Update is available on Windows only");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_verified_current_feed_without_legacy_downgrade() {
        assert!(UPDATE_SCRIPT.contains("releases/download/opengpu-prod"));
        assert!(UPDATE_SCRIPT.contains("strict installer verification failed"));
        assert!(UPDATE_SCRIPT.contains("Stop-InstalledOpenGpuProcesses"));
        assert!(!UPDATE_SCRIPT.contains("cli-windows-v0.2.08"));
        let args = update_arguments(std::path::Path::new("C:\\Temp\\install.ps1"));
        for flag in [
            "-SkipModelRuntime",
            "-SkipContributorSetup",
            "-SkipChatConnect",
        ] {
            assert!(args.iter().any(|arg| arg == flag));
        }
        assert!(!args.iter().any(|arg| arg == "-AllowUnsignedLocalPreview"));
    }
}
