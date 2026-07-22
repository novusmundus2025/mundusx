#![cfg_attr(windows, windows_subsystem = "windows")]

const INSTALL_SCRIPT: &str = include_str!("../../../install.ps1");

fn installed_cli_path() -> std::path::PathBuf {
    if let Ok(install_dir) = std::env::var("OPENGPU_INSTALL_DIR") {
        let install_dir = install_dir.trim();
        if !install_dir.is_empty() {
            return std::path::PathBuf::from(install_dir).join("opengpu.exe");
        }
    }

    std::env::var("USERPROFILE")
        .map(|profile| {
            std::path::PathBuf::from(profile)
                .join(".opengpu")
                .join("bin")
                .join("opengpu.exe")
        })
        .unwrap_or_else(|_| std::path::PathBuf::from("opengpu.exe"))
}

fn installer_arguments(script_path: &std::path::Path) -> Vec<String> {
    let mut arguments = vec![
        "-NoProfile".to_string(),
        "-ExecutionPolicy".to_string(),
        "Bypass".to_string(),
        "-File".to_string(),
        script_path.display().to_string(),
    ];
    if let Ok(release_base) = std::env::var("MUNDUSX_RELEASE_BASE_URL") {
        let release_base = release_base.trim();
        if !release_base.is_empty() {
            arguments.push("-ReleaseBaseUrl".to_string());
            arguments.push(release_base.to_string());
        }
    }
    arguments
}

fn contributor_setup_command(cli_path: &std::path::Path) -> String {
    let escaped = cli_path.display().to_string().replace('\'', "''");
    format!(
        "& '{escaped}' install; Write-Host ''; Write-Host 'Contributor setup finished. Run opengpu start when you are ready to contribute.'; Write-Host 'Press Enter to close this window.'; Read-Host"
    )
}

fn run_installer() -> Result<(), String> {
    let staging = std::env::temp_dir().join(format!(
        "mundusx-setup-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_millis()
    ));
    std::fs::create_dir_all(&staging)
        .map_err(|error| format!("failed to create installer staging directory: {error}"))?;
    let script_path = staging.join("install.ps1");
    std::fs::write(&script_path, INSTALL_SCRIPT)
        .map_err(|error| format!("failed to stage the MundusX installer: {error}"))?;

    let result = std::process::Command::new("powershell.exe")
        .args(installer_arguments(&script_path))
        .status()
        .map_err(|error| format!("failed to start the MundusX installer: {error}"))?;
    let _ = std::fs::remove_dir_all(&staging);
    if result.success() {
        Ok(())
    } else {
        Err(format!(
            "MundusX installation failed with exit code {}",
            result.code().unwrap_or(-1)
        ))
    }
}

fn launch_contributor_setup() -> Result<(), String> {
    let cli_path = installed_cli_path();
    if !cli_path.is_file() {
        return Err(format!(
            "installed opengpu was not found at {}",
            cli_path.display()
        ));
    }

    std::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-NoExit",
            "-Command",
            &contributor_setup_command(&cli_path),
        ])
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("failed to launch contributor setup: {error}"))
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
    message(
        "MundusX Setup",
        "MundusX will download and verify the CLI, node agent, tray application, and the right GPU runtime for this PC. NVIDIA systems use CUDA; other Windows GPU systems use Vulkan. After installation, contributor setup will open so you can choose your control plane, contribution cap, and model.",
        false,
    );
    match run_installer() {
        Ok(()) => match launch_contributor_setup() {
            Ok(()) => message(
                "MundusX Setup",
                "MundusX was installed successfully. The contributor setup wizard is open.",
                false,
            ),
            Err(error) => message(
                "MundusX Setup",
                &format!(
                    "MundusX was installed successfully, but contributor setup could not open automatically.\n\n{error}\n\nRun `opengpu install` manually."
                ),
                true,
            ),
        },
        Err(error) => message("MundusX Setup failed", &error, true),
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("MundusX-Setup is available on Windows only");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embeds_the_windows_bootstrapper() {
        assert!(INSTALL_SCRIPT.contains("MundusX Windows installer"));
        assert!(INSTALL_SCRIPT.contains("mundusx-tray"));
        assert!(INSTALL_SCRIPT.contains("runtime selection"));
        assert!(INSTALL_SCRIPT.contains("llama-server.exe"));
    }

    #[test]
    fn powershell_arguments_use_the_embedded_script() {
        let arguments = installer_arguments(std::path::Path::new("C:\\Temp\\install.ps1"));
        assert!(arguments.windows(2).any(|pair| pair == ["-ExecutionPolicy", "Bypass"]));
        assert!(arguments.windows(2).any(|pair| pair == ["-File", "C:\\Temp\\install.ps1"]));
    }

    #[test]
    fn contributor_setup_command_runs_installed_cli_install() {
        let command = contributor_setup_command(std::path::Path::new(
            "C:\\Users\\tester\\.opengpu\\bin\\opengpu.exe",
        ));
        assert!(command.contains("& 'C:\\Users\\tester\\.opengpu\\bin\\opengpu.exe' install"));
        assert!(command.contains("Contributor setup finished"));
    }
}
