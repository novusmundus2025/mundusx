use crate::storage::AgentConfig;
use std::process::{Child, Command, Stdio};
pub struct MediaProcess(Option<Child>, std::time::Instant);
impl MediaProcess {
    pub fn maintain(&mut self, config: &AgentConfig) {
        let running = self
            .0
            .as_mut()
            .is_some_and(|child| matches!(child.try_wait(), Ok(None)));
        if !running && self.1.elapsed() >= std::time::Duration::from_secs(30) {
            *self = Self::start(config);
        }
    }

    pub fn start(config: &AgentConfig) -> Self {
        if !config
            .contribution
            .operations
            .contains(&crate::contribution_contract::Operation::TextToVideo)
        {
            return Self(None, std::time::Instant::now());
        }
        let server = std::env::var("MUNDUSX_MEDIA_SERVER_URL").ok().or_else(|| {
            ["https://control.mundusx.ai"]
                .contains(&config.control_plane_url.trim_end_matches('/'))
                .then(|| "https://chat.mundusx.ai".into())
        });
        let Some(server) = server else {
            eprintln!("mediaWorker: set MUNDUSX_MEDIA_SERVER_URL for this custom control plane");
            return Self(None, std::time::Instant::now());
        };
        let cli = std::env::var_os("OPENGPU_CLI_EXE")
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::current_exe().ok().map(|p| {
                    p.with_file_name(if cfg!(windows) {
                        "opengpu.exe"
                    } else {
                        "opengpu"
                    })
                })
            });
        let Some(cli) = cli else {
            return Self(None, std::time::Instant::now());
        };
        let mut command = Command::new(cli);
        command
            .args(["media", "serve", "--server", &server])
            .env("OPENGPU_MEDIA_MANAGED", "true")
            .stdin(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        match command.spawn() {
            Ok(child) => Self(Some(child), std::time::Instant::now()),
            Err(e) => {
                eprintln!("mediaWorker: unable to start ({e})");
                Self(None, std::time::Instant::now())
            }
        }
    }
}
impl Drop for MediaProcess {
    fn drop(&mut self) {
        if let Some(child) = self.0.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
