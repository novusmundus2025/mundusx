//! Read-only model-weight cache accounting. Never combine retry files or infer
//! overall startup completion from downloaded bytes.
use std::{fs, io::Read, path::{Path, PathBuf}, sync::mpsc, thread, time::{Duration, Instant}};
use serde_json::Value;

#[derive(Clone, Debug)]
struct Weight { oid: String, size: u64 }

fn weights(metadata: &Value) -> Option<Vec<Weight>> {
    let mut result = Vec::new();
    for item in metadata["siblings"].as_array()? {
        let name = item["rfilename"].as_str()?;
        if name.contains('/') || !name.ends_with(".safetensors") { continue; }
        let oid = item["lfs"]["sha256"].as_str()?;
        let size = item["size"].as_u64()?;
        if oid.len() != 64 || !oid.bytes().all(|b| b.is_ascii_hexdigit()) || size == 0 { return None; }
        if result.iter().any(|w: &Weight| w.oid == oid) { continue; }
        result.push(Weight { oid: oid.into(), size });
    }
    (!result.is_empty()).then_some(result)
}

fn fetch(model: &str, cache: &Path) -> Option<Vec<Weight>> {
    let revision = fs::read_to_string(cache.join("refs/main")).ok()
        .map(|s| s.trim().to_owned()).filter(|s| s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit()));
    let url = format!("https://huggingface.co/api/models/{model}/revision/{}?blobs=true", revision.as_deref().unwrap_or("main"));
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(10)).redirects(0).build();
    let mut request = agent.get(&url);
    if let Ok(token) = std::env::var("HF_TOKEN") { request = request.set("Authorization", &format!("Bearer {token}")); }
    let response = request.call().ok()?;
    let mut data = Vec::new();
    response.into_reader().take(2 * 1024 * 1024 + 1).read_to_end(&mut data).ok()?;
    if data.len() > 2 * 1024 * 1024 { return None; }
    weights(&serde_json::from_slice::<Value>(&data).ok()?)
}

fn stored_bytes(metadata: &fs::Metadata) -> u64 {
    #[cfg(unix)]
    { use std::os::unix::fs::MetadataExt; metadata.len().min(metadata.blocks().saturating_mul(512)) }
    // A partial file's logical size can be preallocated. Don't invent a count
    // on hosts where allocated bytes are unavailable.
    #[cfg(not(unix))]
    { let _ = metadata; 0 }
}

fn totals(blobs: &Path, weights: &[Weight]) -> (u64, u64, usize) {
    let files: Vec<_> = fs::read_dir(blobs).into_iter().flatten().filter_map(Result::ok).collect();
    let mut received = 0u64;
    let mut total = 0u64;
    let mut complete = 0;
    for weight in weights {
        total = total.saturating_add(weight.size);
        let final_file = fs::symlink_metadata(blobs.join(&weight.oid)).ok();
        if final_file.is_some_and(|m| m.is_file() && m.len() == weight.size) {
            received = received.saturating_add(weight.size);
            complete += 1;
            continue;
        }
        // Retries may leave several incomplete files for one blob. Use the
        // newest candidate; adding them would falsely claim >100% downloaded.
        let newest = files.iter().filter_map(|entry| {
            let name = entry.file_name(); let name = name.to_str()?;
            if !name.starts_with(&format!("{}.", weight.oid)) || !name.ends_with(".incomplete") { return None; }
            let metadata = fs::symlink_metadata(entry.path()).ok()?;
            if !metadata.is_file() { return None; }
            Some((metadata.modified().ok()?, stored_bytes(&metadata).min(weight.size)))
        }).max_by_key(|(modified, _)| *modified);
        received = received.saturating_add(newest.map(|(_, bytes)| bytes).unwrap_or(0));
    }
    (received, total, complete)
}

fn label(received: u64, total: u64, complete: usize, count: usize) -> String {
    let percent = if complete == count { 100 } else { received.saturating_mul(100).checked_div(total).unwrap_or(0).min(99) };
    let filled = percent as usize / 5;
    let suffix = if complete == count { "download complete; model loading/warmup still pending" } else { "estimated cached bytes; download only" };
    format!("Model download: [{}{}] {percent}% ({:.2}/{:.2} GiB; {complete}/{count} weight files complete; {suffix})", "#".repeat(filled), "-".repeat(20-filled), received as f64 / 1073741824.0, total as f64 / 1073741824.0)
}

pub struct DownloadProgress {
    model: String,
    cache: PathBuf,
    pending: Option<mpsc::Receiver<Option<Vec<Weight>>>>,
    weights: Option<Vec<Weight>>,
    next_fetch: Instant,
    next_sample: Instant,
    done: bool,
}
impl DownloadProgress {
    pub fn new(model: &str, model_dir: &Path) -> Option<Self> {
        let parts: Vec<_> = model.split('/').collect();
        if parts.len() != 2 || parts.iter().any(|s| s.is_empty() || *s == "." || *s == ".." || !s.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))) { return None; }
        Some(Self { model: model.into(), cache: model_dir.join(".huggingface/hub").join(format!("models--{}", model.replace('/', "--"))), pending: None, weights: None, next_fetch: Instant::now(), next_sample: Instant::now(), done: false })
    }
    pub fn poll(&mut self) -> Option<String> {
        if self.done { return None; }
        if let Some(receiver) = &self.pending {
            match receiver.try_recv() {
                Ok(value) => {
                    self.weights = value; self.pending = None;
                    if self.weights.is_none() { return Some("Downloading model weights: byte total unavailable; retrying metadata shortly".into()); }
                }
                Err(mpsc::TryRecvError::Disconnected) => { self.pending = None; }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if self.weights.is_none() && self.pending.is_none() && Instant::now() >= self.next_fetch {
            let (sender, receiver) = mpsc::channel();
            let model = self.model.clone(); let cache = self.cache.clone();
            thread::spawn(move || { let _ = sender.send(fetch(&model, &cache)); });
            self.pending = Some(receiver);
            self.next_fetch = Instant::now() + Duration::from_secs(30);
            return Some("Downloading model weights: fetching expected byte total (percentage not available yet)".into());
        }
        let weights = self.weights.as_ref()?;
        if Instant::now() < self.next_sample { return None; }
        self.next_sample = Instant::now() + Duration::from_secs(2);
        let (received, total, complete) = totals(&self.cache.join("blobs"), weights);
        self.done = complete == weights.len();
        Some(label(received, total, complete, weights.len()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn download_progress_does_not_count_retries_twice_or_report_early_completion() {
        let dir = std::env::temp_dir().join(format!("weight-progress-{}", uuid::Uuid::new_v4())); fs::create_dir_all(&dir).unwrap();
        let oid = "a".repeat(64); let weights = vec![Weight { oid: oid.clone(), size: 100 }];
        fs::write(dir.join(format!("{oid}.old.incomplete")), vec![0; 80]).unwrap();
        thread::sleep(Duration::from_millis(20));
        fs::write(dir.join(format!("{oid}.new.incomplete")), vec![0; 40]).unwrap();
        let (received,total,complete) = totals(&dir,&weights);
        #[cfg(unix)] assert_eq!(received,40);
        assert_eq!((total,complete),(100,0));
        assert!(label(100,100,0,1).contains("99%"));
        fs::write(dir.join(&oid),vec![0;100]).unwrap();
        assert_eq!(totals(&dir,&weights),(100,100,1));
        assert!(label(100,100,1,1).contains("loading/warmup still pending"));
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn download_progress_parses_expected_weight_sizes_only() {
        let value = serde_json::json!({"siblings":[
            {"rfilename":"config.json","size":123},
            {"rfilename":"model-00001-of-00002.safetensors","size":28176516456u64,"lfs":{"sha256":"a".repeat(64)}},
            {"rfilename":"model-00002-of-00002.safetensors","size":6216318704u64,"lfs":{"sha256":"b".repeat(64)}}
        ]});
        let expected = weights(&value).unwrap();
        assert_eq!(expected.iter().map(|w|w.size).sum::<u64>(),34392835160);
        assert!(label(32752129186,34392835160,1,2).contains("95%"));
    }
    #[cfg(unix)]
    #[test]
    fn download_progress_does_not_count_sparse_preallocation() {
        let path = std::env::temp_dir().join(format!("sparse-progress-{}",uuid::Uuid::new_v4()));
        let file = fs::File::create(&path).unwrap(); file.set_len(1_000_000).unwrap();
        assert!(stored_bytes(&file.metadata().unwrap()) < 1_000_000);
        drop(file); fs::remove_file(path).unwrap();
    }
    #[test]
    fn download_progress_rejects_invalid_metadata_and_paths() {
        assert!(DownloadProgress::new("../model",Path::new(".")).is_none());
        assert!(weights(&serde_json::json!({"siblings":[{"rfilename":"model.safetensors","size":1,"lfs":{"sha256":"../secret"}}]})).is_none());
    }
}
