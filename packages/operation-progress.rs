//! Human-readable activity on stderr; never write progress into JSON stdout.
use std::sync::mpsc::{self, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub struct OperationProgress {
    stop: Option<Sender<()>>,
    worker: Option<JoinHandle<()>>,
}
impl OperationProgress {
    pub fn start(label: impl Into<String>) -> Self {
        Self::with_interval(label.into(), Duration::from_secs(10))
    }
    fn with_interval(label: String, interval: Duration) -> Self {
        eprintln!("operationProgress: {label}: starting");
        let (stop, receiver) = mpsc::channel();
        let worker = thread::spawn(move || {
            let started = Instant::now();
            while receiver.recv_timeout(interval) == Err(mpsc::RecvTimeoutError::Timeout) {
                eprintln!("operationProgress: {label}: still running ({}s elapsed)", started.elapsed().as_secs());
            }
        });
        Self { stop: Some(stop), worker: Some(worker) }
    }
}
impl Drop for OperationProgress {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() { let _ = stop.send(()); }
        if let Some(worker) = self.worker.take() { let _ = worker.join(); }
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
    fn error_return_drops_activity_worker() {
        fn failure() -> Result<(), ()> {
            let _progress = OperationProgress::start("test failure");
            Err(())
        }
        assert!(failure().is_err());
    }
}
