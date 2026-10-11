//! Human-readable activity on stderr; never write progress into JSON stdout.
use std::sync::{mpsc::{self, Sender}, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use std::io::{self, IsTerminal, Write};

fn terminal_progress() -> bool {
    std::env::var("OPENGPU_PROGRESS_RENDER").ok().as_deref() == Some("terminal")
        || (io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none() && std::env::var_os("CI").is_none())
}
fn show_stage(label: &str) {
    if terminal_progress() {
        let columns = std::env::var("OPENGPU_PROGRESS_COLUMNS").ok().and_then(|s| s.parse::<usize>().ok()).unwrap_or(80).max(20);
        let label = label.chars().take(columns-1).collect::<String>();
        let mut out = io::stderr().lock();
        let _ = write!(out, "\r\x1b[2K\x1b[38;2;76;172;249m{label}\x1b[0m");
        let _ = out.flush();
    } else { eprintln!("operationProgress: {label}"); }
}

pub struct OperationProgress {
    stop: Option<Sender<()>>,
    worker: Option<JoinHandle<()>>,
    stage: Arc<Mutex<(String, Instant)>>,
}
impl OperationProgress {
    pub fn start(label: impl Into<String>) -> Self {
        Self::with_interval(label.into(), Duration::from_secs(10))
    }
    /// Replace the current stage, retaining one activity reporter per operation.
    pub fn set_stage(&self, label: impl Into<String>) {
        let label = label.into();
        let mut stage = self.stage.lock().unwrap_or_else(|e| e.into_inner());
        if stage.0 != label {
            show_stage(&label);
            *stage = (label, Instant::now());
        }
    }
    fn with_interval(label: String, interval: Duration) -> Self {
        show_stage(&label);
        let stage = Arc::new(Mutex::new((label, Instant::now())));
        let current = stage.clone();
        let (stop, receiver) = mpsc::channel();
        let worker = thread::spawn(move || {
            while receiver.recv_timeout(interval) == Err(mpsc::RecvTimeoutError::Timeout) {
                let stage = current.lock().unwrap_or_else(|e| e.into_inner());
                if stage.1.elapsed() >= interval {
                    show_stage(&format!("{} ({}s since last progress; waiting for next update)", stage.0, stage.1.elapsed().as_secs()));
                }
            }
        });
        Self { stop: Some(stop), worker: Some(worker), stage }
    }
}
impl Drop for OperationProgress {
    fn drop(&mut self) {
        drop(self.stop.take());
        if let Some(worker) = self.worker.take() { let _ = worker.join(); }
        if terminal_progress() { eprintln!(); }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_does_not_wait_for_the_next_activity_interval() {
        let progress = OperationProgress::with_interval("test".into(), Duration::from_secs(60));
        let started = Instant::now();
        drop(progress);
        assert!(started.elapsed() < Duration::from_secs(2));
    }
    #[test]
    fn stage_changes_replace_previous_activity_without_resetting_duplicates() {
        let progress = OperationProgress::start("Downloading runtime");
        progress.set_stage("Loading model");
        let changed = progress.stage.lock().unwrap().1;
        progress.set_stage("Loading model");
        let stage = progress.stage.lock().unwrap();
        assert_eq!(stage.0, "Loading model");
        assert_eq!(stage.1, changed);
    }
    #[test]
    fn error_return_drops_activity_worker() {
        fn failure() -> Result<(), ()> {
            let _progress = OperationProgress::start("test failure");
            Err(())
        }
        assert!(failure().is_err());
    }
}
