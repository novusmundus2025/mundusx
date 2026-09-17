#![cfg_attr(windows, windows_subsystem = "windows")]

const UPDATE_SCRIPT: &str = include_str!("../../../update-windows-0.1.95.ps1");

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
            "MundusX update failed with exit code {}. See .opengpu\\logs\\update-0.1.95.log for details.",
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
            "MundusX 0.1.95 is installed. Return to Chat and refresh the page.",
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
        assert!(UPDATE_SCRIPT.contains("cli-windows-v0.1.95"));
        assert!(UPDATE_SCRIPT.contains("bbcb41f01fe81dd26d3595488982ae24c08c3925641c1bc5d64de582f9be56a2"));
        assert!(UPDATE_SCRIPT.contains("255460c5434ddb321e53ba6d73e552b6d1441f6789379a36dde1f3fc7c78b900"));
        assert!(UPDATE_SCRIPT.contains("mundusx-tray.exe"));
    }
}
