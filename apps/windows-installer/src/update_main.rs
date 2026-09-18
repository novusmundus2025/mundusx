#![cfg_attr(windows, windows_subsystem = "windows")]

const UPDATE_SCRIPT: &str = include_str!("../../../update-windows-0.2.00.ps1");

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
            "MundusX update failed with exit code {}. See .opengpu\\logs\\update-0.2.00.log for details.",
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
            "MundusX 0.2.00 is installed. Return to Chat and refresh the page.",
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
        assert!(UPDATE_SCRIPT.contains("cli-windows-v0.2.00"));
        assert!(UPDATE_SCRIPT.contains("e8d7f0d97a2577bcbc292ba91eb2da3d1a29fe6e4ee65cfa91287bb614cf71ce"));
        assert!(UPDATE_SCRIPT.contains("4975e1e2ac413a1a05387efb812d94ea3b3f3b5d9cf9d90a354a48b1fc291778"));
        assert!(UPDATE_SCRIPT.contains("mundusx-tray.exe"));
    }
}
