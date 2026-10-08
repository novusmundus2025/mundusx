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
        if !media_selected(config) {
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
            .env("OPENGPU_MEDIA_PARENT_PID", std::process::id().to_string())
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
            let running = matches!(child.try_wait(), Ok(None));
            let _ = child.kill();
            let _ = child.wait();
            if running {
              if let Err(e) = cleanup_orphan_runtime() {
                eprintln!("mediaWorker: {e}; managed lease will expire if the worker stopped");
              }
            }
        }
    }
}

pub fn cleanup_orphan_runtime() -> Result<(), String> {
    let home = crate::storage::config_dir();
    if !home.join("media/active-container.json").exists() { return Ok(()); }
    if crate::media_drain::active(&home) { return Err("active generation owns the media slot".into()); }
    crate::media_runtime::run_profile(&home, 70, None, "stop", false, &[])
}

fn media_selected(config: &AgentConfig) -> bool {
    config.contribution.operations.iter().any(|operation| {
        matches!(
            operation,
            crate::contribution_contract::Operation::ImageToVideo
                | crate::contribution_contract::Operation::TextToVideo
                | crate::contribution_contract::Operation::TextToImage
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contribution_contract::Operation;
    #[test]
    fn starts_for_image_or_video_but_not_llm_or_editing_only() {
        let mut cfg = AgentConfig::default();
        for (operation, expected) in [
            (Operation::TextToImage, true),
            (Operation::TextToVideo, true),
            (Operation::ImageToVideo, true),
            (Operation::Llm, false),
            (Operation::ImageEdit, false),
        ] {
            cfg.contribution.operations = vec![operation];
            assert_eq!(media_selected(&cfg), expected);
        }
    }
}
