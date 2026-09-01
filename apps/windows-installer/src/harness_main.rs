#![cfg_attr(windows, windows_subsystem = "windows")]

const INSTALL_SCRIPT: &str = include_str!("../../../install-harness-runner.ps1");

fn stage_and_run() -> Result<(), String> {
    let directory =
        std::env::temp_dir().join(format!("mundusx-harness-setup-{}", std::process::id()));
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("Could not prepare the installer: {error}"))?;
    let script = directory.join("install-harness-runner.ps1");
    std::fs::write(&script, INSTALL_SCRIPT)
        .map_err(|error| format!("Could not prepare the installer: {error}"))?;
    let chat_url =
        std::env::var("MUNDUSX_CHAT_URL").unwrap_or_else(|_| "https://chat.mundusx.ai".to_string());
    let status = std::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            &script.display().to_string(),
            "-ChatUrl",
            &chat_url,
        ])
        .status()
        .map_err(|error| format!("Could not start the installer: {error}"))?;
    let _ = std::fs::remove_dir_all(directory);
    status.success().then_some(()).ok_or_else(|| {
        format!(
            "Runner setup exited with code {}",
            status.code().unwrap_or(-1)
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
    message(
        "Connect MundusX runner",
        "This installs only the user-owned coding runner. It does not install an inference node, models, or GPU runtimes. Your browser will open once for approval.",
        false,
    );
    match stage_and_run() {
        Ok(()) => message(
            "MundusX runner connected",
            "This computer is connected. Return to Chat; your coding request will resume automatically.",
            false,
        ),
        Err(error) => message("MundusX runner setup failed", &error, true),
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("MundusX Harness setup is available on Windows only");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installer_is_runner_only_and_verifies_checksum() {
        assert!(INSTALL_SCRIPT.contains("mundusx-harness-runner"));
        assert!(INSTALL_SCRIPT.contains("Get-FileHash"));
        assert!(!INSTALL_SCRIPT.contains("llama-runtime"));
        assert!(!INSTALL_SCRIPT.contains("opengpu-node-agent"));
    }
}
