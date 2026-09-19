#![cfg_attr(windows, windows_subsystem = "windows")]

const UPDATE_SCRIPT: &str = include_str!("../../../update-windows-0.2.08.ps1");

fn run_update() -> Result<(), String> {
    let directory = std::env::temp_dir().join(format!("mundusx-update-{}", std::process::id()));
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("Could not prepare the update: {error}"))?;
    let script = directory.join("update.ps1");
    std::fs::write(&script, UPDATE_SCRIPT)
        .map_err(|error| format!("Could not prepare the update: {error}"))?;
    let result = std::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            &script.display().to_string(),
        ])
        .status()
        .map_err(|error| format!("Could not start the update: {error}"))?;
    let _ = std::fs::remove_dir_all(&directory);
    result.success().then_some(()).ok_or_else(|| {
        format!(
            "MundusX update failed with exit code {}. See .opengpu\\logs\\update-0.2.08.log for details.",
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
            MB_OK | if error { MB_ICONERROR } else { MB_ICONINFORMATION },
        );
    }
}

#[cfg(windows)]
fn main() {
    match run_update() {
        Ok(()) => message(
            "MundusX updated",
            "MundusX 0.2.08 is installed. Return to Chat and refresh the page.",
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
    fn embeds_versioned_verified_update() {
        assert!(UPDATE_SCRIPT.contains("cli-windows-v0.2.08"));
        assert!(UPDATE_SCRIPT.contains("4a822dfdd4c0df90dd6d55812f85d61973e081538fe79999d6189543c63aad2f"));
        assert!(UPDATE_SCRIPT.contains("4e1d83d87777d0d6e6e1f4956ad932b0354eefbebea13e24ef59bdad328bce40"));
        assert!(UPDATE_SCRIPT.contains("opengpu-node-agent-x86_64-pc-windows-msvc.exe"));
        assert!(UPDATE_SCRIPT.contains("mundusx-agent-server-x86_64-pc-windows-msvc.exe"));
        assert!(UPDATE_SCRIPT.contains("mundusx-tray.exe"));
    }
}
