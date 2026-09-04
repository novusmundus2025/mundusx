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

fn installer_log_path() -> std::path::PathBuf {
    std::env::var("USERPROFILE")
        .map(|profile| {
            std::path::PathBuf::from(profile)
                .join(".opengpu")
                .join("logs")
                .join("installer.log")
        })
        .unwrap_or_else(|_| std::env::temp_dir().join("mundusx-installer.log"))
}

fn installer_arguments(script_path: &std::path::Path, agent_mode: &str) -> Vec<String> {
    let mut arguments = vec![
        "-NoProfile".to_string(),
        "-ExecutionPolicy".to_string(),
        "Bypass".to_string(),
        "-File".to_string(),
        script_path.display().to_string(),
        "-SkipContributorSetup".to_string(),
        "-AgentMode".to_string(),
        agent_mode.to_string(),
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
        "& '{escaped}' install; $setupExit = $LASTEXITCODE; Write-Host ''; if ($setupExit -eq 0) {{ Write-Host 'Contributor setup finished. Run opengpu start when you are ready to contribute.' -ForegroundColor Green }} else {{ Write-Host 'Contributor setup failed. Review the error above.' -ForegroundColor Red }}"
    )
}

fn run_installer(agent_mode: &str) -> Result<(), String> {
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

    let log_path = installer_log_path();
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create installer log directory: {error}"))?;
    }
    let result = std::process::Command::new("powershell.exe")
        .args(installer_arguments(&script_path, agent_mode))
        .status()
        .map_err(|error| format!("failed to start the MundusX installer: {error}"))?;
    let _ = std::fs::remove_dir_all(&staging);
    if result.success() {
        Ok(())
    } else {
        Err(format!(
            "MundusX installation failed with exit code {}.\n\nDetails were saved to:\n{}",
            result.code().unwrap_or(-1),
            log_path.display()
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

    let mut command = std::process::Command::new("powershell.exe");
    command.args([
        "-NoLogo",
        "-NoProfile",
        "-NoExit",
        "-ExecutionPolicy",
        "Bypass",
        "-Command",
        &contributor_setup_command(&cli_path),
    ]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
        command.creation_flags(CREATE_NEW_CONSOLE);
    }
    command
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
fn choose_agent() -> Option<&'static str> {
    use std::{ffi::OsStr, os::windows::ffi::OsStrExt, ptr};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, IDCANCEL, IDNO, IDYES, MB_ICONQUESTION, MB_YESNOCANCEL,
    };
    let title: Vec<u16> = OsStr::new("MundusX Agent")
        .encode_wide()
        .chain(Some(0))
        .collect();
    let body: Vec<u16> = OsStr::new(
        "Choose the local agent harness:\n\nYes — Hermes Agent (recommended)\nNo — MundusX Agent (built in)\nCancel — No local agent (chat and contributor only)\n\nMundusX still manages models and contributed compute for every choice."
    ).encode_wide().chain(Some(0)).collect();
    let result = unsafe {
        MessageBoxW(
            ptr::null_mut(),
            body.as_ptr(),
            title.as_ptr(),
            MB_YESNOCANCEL | MB_ICONQUESTION,
        )
    };
    match result {
        IDYES => Some("hermes"),
        IDNO => Some("native"),
        IDCANCEL => Some("none"),
        _ => None,
    }
}

#[cfg(windows)]
fn main() {
    message(
        "MundusX Setup",
        "MundusX will download and verify the CLI, node agent, tray application, and the right GPU runtime for this PC. NVIDIA systems use CUDA; other Windows GPU systems use Vulkan. After installation, contributor setup will open so you can choose your control plane, contribution cap, and model.",
        false,
    );
    let Some(agent_mode) = choose_agent() else {
        return;
    };
    match run_installer(agent_mode) {
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
        assert!(INSTALL_SCRIPT.contains("Start-ContributorSetup"));
        assert!(INSTALL_SCRIPT.contains("Stop-InstalledOpenGpuProcesses"));
        assert!(!INSTALL_SCRIPT.contains("Move-Item -Force -Path $tempTray"));
        assert!(INSTALL_SCRIPT.contains("-NoExit"));
        assert!(INSTALL_SCRIPT.contains("llama-server.exe"));
        assert!(INSTALL_SCRIPT
            .contains("https://github.com/mundusx/releases/releases/download/opengpu-prod"));
        assert!(!INSTALL_SCRIPT.contains("github.com/mundusx/mundusx/releases/latest"));
    }

    #[test]
    fn powershell_arguments_use_the_embedded_script() {
        let arguments =
            installer_arguments(std::path::Path::new("C:\\Temp\\install.ps1"), "hermes");
        assert!(arguments
            .windows(2)
            .any(|pair| pair == ["-ExecutionPolicy", "Bypass"]));
        assert!(arguments
            .windows(2)
            .any(|pair| pair == ["-File", "C:\\Temp\\install.ps1"]));
        assert!(arguments
            .iter()
            .any(|argument| argument == "-SkipContributorSetup"));
        assert!(arguments
            .windows(2)
            .any(|pair| pair == ["-AgentMode", "hermes"]));
    }

    #[test]
    fn contributor_setup_command_runs_installed_cli_install() {
        let command = contributor_setup_command(std::path::Path::new(
            "C:\\Users\\tester\\.opengpu\\bin\\opengpu.exe",
        ));
        assert!(command.contains("& 'C:\\Users\\tester\\.opengpu\\bin\\opengpu.exe' install"));
        assert!(command.contains("Contributor setup finished"));
        assert!(command.contains("if ($setupExit -eq 0)"));
        assert!(!command.contains("Start-Sleep"));
        assert!(!command.contains("Read-Host"));
    }

    #[test]
    fn installer_log_is_kept_under_the_opengpu_home() {
        assert!(installer_log_path().ends_with(".opengpu\\logs\\installer.log"));
    }
}
