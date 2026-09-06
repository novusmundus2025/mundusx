//! Detection of a local LLM cluster that is already running on this host.
//!
//! `opengpu install` and `opengpu start` probe well-known localhost inference
//! endpoints before provisioning anything. When one answers, the contributor is
//! asked whether that running cluster should be contributed to MundusX instead
//! of standing up a second runtime and downloading another copy of the weights.

use serde::{Deserialize, Serialize};
use std::env;
use std::time::Duration;

const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// Port owned by the MundusX-managed `llama-server`. A server answering here is
/// our own runtime, not a foreign cluster the contributor already runs.
const MANAGED_LLAMA_SERVER_PORT: u16 = 8789;

/// Comma-separated base URLs that replace the default probe list.
const PROBE_URLS_ENV: &str = "OPENGPU_CLUSTER_PROBE_URLS";
/// Set to `1` to skip cluster detection entirely (CI, scripted installs).
const SKIP_DETECT_ENV: &str = "OPENGPU_SKIP_CLUSTER_DETECT";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ClusterKind {
    Ollama,
    LmStudio,
    Vllm,
    LlamaCpp,
    OpenAiCompatible,
}

impl ClusterKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ollama => "ollama",
            Self::LmStudio => "lm-studio",
            Self::Vllm => "vllm",
            Self::LlamaCpp => "llama.cpp",
            Self::OpenAiCompatible => "openai-compatible",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Ollama => "Ollama",
            Self::LmStudio => "LM Studio",
            Self::Vllm => "vLLM",
            Self::LlamaCpp => "llama.cpp",
            Self::OpenAiCompatible => "OpenAI-compatible server",
        }
    }

    fn primary_path(self) -> &'static str {
        match self {
            Self::Ollama => "/api/tags",
            _ => "/v1/models",
        }
    }

    /// Which listing path to try first. A port is only a hint: the runtime is
    /// identified from what the endpoint actually returns.
    fn probe_hint(base_url: &str) -> Self {
        match port_of(base_url) {
            Some(11434) => Self::Ollama,
            Some(1234) => Self::LmStudio,
            Some(8000) => Self::Vllm,
            _ => Self::OpenAiCompatible,
        }
    }
}

/// One model a detected endpoint advertises, with whatever size signal the
/// runtime reported. Size is what ranks clusters against each other, so a
/// contributor with several runtimes up is offered the biggest one first.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelInfo {
    pub name: String,
    /// Parameter count, e.g. llama.cpp `meta.n_params` or Ollama `8.0B`.
    pub params: Option<u64>,
    /// On-disk size in bytes, when the runtime reports it.
    pub bytes: Option<u64>,
    /// Capabilities the runtime advertises, e.g. `completion`, `tools`.
    pub capabilities: Vec<String>,
    /// Trained context length: llama.cpp `meta.n_ctx_train` or Ollama
    /// `details.context_length`.
    pub context_tokens: Option<u32>,
}

impl ModelInfo {
    pub fn new(name: impl Into<String>, params: Option<u64>, bytes: Option<u64>) -> Self {
        Self {
            name: name.into(),
            params,
            bytes,
            capabilities: Vec::new(),
            context_tokens: None,
        }
    }

    fn has_capability(&self, needle: &str) -> bool {
        self.capabilities
            .iter()
            .any(|value| value.to_ascii_lowercase().contains(needle))
    }

    /// The runtime advertises tool/function calling.
    pub fn supports_tools(&self) -> bool {
        self.has_capability("tool")
    }

    /// An embedding model, by advertised capability or by name.
    pub fn supports_embeddings(&self) -> bool {
        self.has_capability("embed") || self.name.to_ascii_lowercase().contains("embed")
    }

    pub fn supports_vision(&self) -> bool {
        self.has_capability("vision")
    }

    /// Ranking key: parameter count dominates, on-disk bytes break ties. A
    /// model that reports neither sorts last rather than being dropped.
    pub fn size_key(&self) -> (u64, u64) {
        (self.params.unwrap_or(0), self.bytes.unwrap_or(0))
    }

    /// Human-readable size, or `None` when the runtime reported no signal.
    pub fn size_label(&self) -> Option<String> {
        match (self.params, self.bytes) {
            (Some(params), Some(bytes)) => Some(format!(
                "{}, {}",
                format_params(params),
                format_bytes(bytes)
            )),
            (Some(params), None) => Some(format_params(params)),
            (None, Some(bytes)) => Some(format_bytes(bytes)),
            (None, None) => None,
        }
    }

    /// `name (753B, 222.2 GB)`, or just the name when size is unknown.
    pub fn label(&self) -> String {
        match self.size_label() {
            Some(size) => format!("{} ({})", self.name, size),
            None => self.name.clone(),
        }
    }
}

pub fn format_params(params: u64) -> String {
    if params >= 1_000_000_000 {
        let billions = params as f64 / 1_000_000_000.0;
        if billions >= 100.0 {
            format!("{billions:.0}B params")
        } else {
            format!("{billions:.1}B params")
        }
    } else if params >= 1_000_000 {
        format!("{:.0}M params", params as f64 / 1_000_000.0)
    } else {
        format!("{params} params")
    }
}

pub fn format_bytes(bytes: u64) -> String {
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;
    let gb = bytes as f64 / GB;
    if gb >= 1.0 {
        format!("{gb:.1} GB")
    } else {
        format!("{:.0} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// A local inference endpoint that answered a model-listing probe.
#[derive(Clone, Debug, PartialEq)]
pub struct DetectedCluster {
    pub kind: ClusterKind,
    pub base_url: String,
    pub models: Vec<ModelInfo>,
    /// Context window the endpoint is actually serving, when it reports one.
    /// This can be far smaller than a model's trained context — a llama.cpp
    /// server started with `-c 256` serves 256 tokens no matter what the GGUF
    /// was trained at — so it is what the node must advertise.
    pub served_context_tokens: Option<u32>,
    /// Fraction of machine memory the runtime was given, as it reports it
    /// (vLLM `gpu_memory_utilization`). This is the share of the machine the
    /// cluster actually occupies, so it is the honest basis for usable memory.
    pub memory_utilization: Option<f32>,
    /// Concurrent full-context sequences the runtime says it can hold
    /// (vLLM `kv_cache_max_concurrency`).
    pub max_concurrency: Option<u32>,
    /// Configured scheduler ceiling (vLLM `max_num_seqs`). This is distinct
    /// from the theoretical KV-cache capacity above.
    pub max_num_seqs: Option<u32>,
    /// Total token capacity of the runtime KV cache, when exported.
    pub kv_cache_tokens: Option<u64>,
    /// True when the runtime has a tool-call parser loaded.
    pub supports_tool_calls: bool,
}

impl DetectedCluster {
    /// The biggest model the endpoint serves. This is what the node advertises,
    /// so a runtime holding both a 70B and an 8B contributes the 70B.
    pub fn largest_model(&self) -> Option<&ModelInfo> {
        self.models.iter().max_by_key(|model| model.size_key())
    }

    pub fn primary_model(&self) -> Option<&str> {
        self.largest_model().map(|model| model.name.as_str())
    }

    pub fn model_names(&self) -> Vec<String> {
        self.models.iter().map(|model| model.name.clone()).collect()
    }

    /// Ranking key for choosing between endpoints: the size of the biggest
    /// model each one serves.
    pub fn size_key(&self) -> (u64, u64) {
        self.largest_model()
            .map(ModelInfo::size_key)
            .unwrap_or((0, 0))
    }

    /// True when the endpoint can actually serve work today. An endpoint that
    /// answers with an empty model list is running but has nothing loaded.
    pub fn is_servable(&self) -> bool {
        !self.models.is_empty()
    }

    pub fn summary(&self) -> String {
        if self.models.is_empty() {
            return format!(
                "{} at {} (no models loaded)",
                self.kind.label(),
                self.base_url
            );
        }
        let mut ranked: Vec<&ModelInfo> = self.models.iter().collect();
        ranked.sort_by_key(|model| std::cmp::Reverse(model.size_key()));
        let shown: Vec<String> = ranked.iter().take(3).map(|model| model.label()).collect();
        let more = self.models.len().saturating_sub(shown.len());
        let suffix = if more > 0 {
            format!(" +{more} more")
        } else {
            String::new()
        };
        format!(
            "{} at {} ({} model{}: {}{})",
            self.kind.label(),
            self.base_url,
            self.models.len(),
            if self.models.len() == 1 { "" } else { "s" },
            shown.join(", "),
            suffix
        )
    }
}

/// What the caller should do about a detected cluster before touching models.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ClusterPromptDecision {
    /// Ask the contributor whether to contribute the running cluster.
    Ask,
    /// A flag already answered yes.
    AutoContribute,
    /// A flag already answered no.
    AutoDecline,
    /// Nothing to ask; the reason is printed as a hint.
    Skip(&'static str),
}

/// Pure decision table so install/start share one policy.
///
/// The forced flags win over every remembered state so re-running with
/// `--contribute-cluster` can re-adopt an endpoint that moved.
pub fn cluster_prompt_decision(
    servable_cluster_found: bool,
    already_contributing: bool,
    previously_declined: bool,
    forced: Option<bool>,
    interactive: bool,
) -> ClusterPromptDecision {
    if let Some(forced) = forced {
        if !servable_cluster_found {
            return ClusterPromptDecision::Skip("no running local cluster was detected");
        }
        return if forced {
            ClusterPromptDecision::AutoContribute
        } else {
            ClusterPromptDecision::AutoDecline
        };
    }

    if !servable_cluster_found {
        return ClusterPromptDecision::Skip("no running local cluster was detected");
    }

    if already_contributing {
        return ClusterPromptDecision::Skip("a running cluster is already contributed");
    }

    if previously_declined {
        return ClusterPromptDecision::Skip(
            "cluster contribution was declined before; run `opengpu cluster scan` to revisit",
        );
    }

    if !interactive {
        return ClusterPromptDecision::Skip(
            "non-interactive run; pass `--contribute-cluster` to contribute the running cluster",
        );
    }

    ClusterPromptDecision::Ask
}

fn port_of(base_url: &str) -> Option<u16> {
    base_url
        .rsplit_once(':')
        .and_then(|(_, port)| port.trim_end_matches('/').parse().ok())
}

pub fn normalize_base_url(raw: &str) -> Option<String> {
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        Some(trimmed.to_string())
    } else {
        None
    }
}

/// Endpoints probed when the environment does not override the list.
fn default_base_urls() -> Vec<String> {
    vec![
        "http://127.0.0.1:11434".to_string(),
        "http://127.0.0.1:1234".to_string(),
        "http://127.0.0.1:8000".to_string(),
        "http://127.0.0.1:8080".to_string(),
    ]
}

fn configured_base_urls() -> Vec<String> {
    match env::var(PROBE_URLS_ENV) {
        Ok(raw) if !raw.trim().is_empty() => raw
            .split(',')
            .filter_map(|value| normalize_base_url(value))
            .collect(),
        _ => default_base_urls(),
    }
}

/// Keeps the MundusX-managed runtime out of the "foreign cluster" set.
fn is_managed_endpoint(base_url: &str) -> bool {
    port_of(base_url) == Some(MANAGED_LLAMA_SERVER_PORT)
}

pub fn detection_disabled() -> bool {
    matches!(env::var(SKIP_DETECT_ENV), Ok(value) if value.trim() == "1")
}

fn http_get_json(url: &str) -> Option<serde_json::Value> {
    ureq::get(url)
        .timeout(PROBE_TIMEOUT)
        .call()
        .ok()
        .filter(|response| response.status() < 400)
        .and_then(|response| response.into_json().ok())
}

fn http_get_text(url: &str) -> Option<String> {
    ureq::get(url)
        .timeout(PROBE_TIMEOUT)
        .call()
        .ok()
        .filter(|response| response.status() < 400)
        .and_then(|response| response.into_string().ok())
}

/// Probe every candidate endpoint and return the ones that answered.
pub fn detect_running_clusters() -> Vec<DetectedCluster> {
    if detection_disabled() {
        return Vec::new();
    }
    detect_with_text(configured_base_urls(), http_get_json, http_get_text)
}

/// Probe a single endpoint the contributor named explicitly.
pub fn probe_cluster(base_url: &str) -> Option<DetectedCluster> {
    let base_url = normalize_base_url(base_url)?;
    detect_with_text(vec![base_url], http_get_json, http_get_text)
        .into_iter()
        .next()
}

/// Detection core with an injectable fetcher so the probe order and parsing can
/// be tested without opening a socket.
pub fn detect_with<F>(base_urls: Vec<String>, fetch: F) -> Vec<DetectedCluster>
where
    F: Fn(&str) -> Option<serde_json::Value>,
{
    detect_with_text(base_urls, fetch, |_| None)
}

/// Detection core with separate JSON and plain-text fetchers, so the
/// Prometheus `/metrics` body can be read without pretending it is JSON.
pub fn detect_with_text<F, G>(
    base_urls: Vec<String>,
    fetch: F,
    fetch_text: G,
) -> Vec<DetectedCluster>
where
    F: Fn(&str) -> Option<serde_json::Value>,
    G: Fn(&str) -> Option<String>,
{
    let mut found: Vec<DetectedCluster> = Vec::new();

    for base_url in base_urls {
        if is_managed_endpoint(&base_url) || found.iter().any(|entry| entry.base_url == base_url) {
            continue;
        }

        let hint = ClusterKind::probe_hint(&base_url);
        let mut paths = vec![hint.primary_path()];
        for candidate in ["/v1/models", "/api/tags"] {
            if !paths.contains(&candidate) {
                paths.push(candidate);
            }
        }

        for path in paths {
            let Some(body) = fetch(&format!("{base_url}{path}")) else {
                continue;
            };
            let served_context_tokens = fetch(&format!("{base_url}/props"))
                .as_ref()
                .and_then(parse_served_context)
                .or_else(|| parse_listing_context(&body));
            let capacity = fetch_text(&format!("{base_url}/metrics"))
                .as_deref()
                .map(parse_server_capacity)
                .unwrap_or_default();
            let max_num_seqs = fetch(&format!("{base_url}/server_info?config_format=json"))
                .or_else(|| fetch(&format!("{base_url}/server_info")))
                .as_ref()
                .and_then(parse_max_num_seqs);
            found.push(DetectedCluster {
                kind: identify_kind(&body, &base_url),
                base_url: base_url.clone(),
                models: parse_models(&body),
                served_context_tokens,
                memory_utilization: capacity.memory_utilization,
                max_concurrency: capacity.max_concurrency,
                max_num_seqs,
                kv_cache_tokens: capacity.kv_cache_tokens,
                supports_tool_calls: capacity.supports_tool_calls,
            });
            break;
        }
    }

    found
}

/// Parses `8.0B` / `70B` / `350M` parameter-size strings as Ollama reports them.
pub fn parse_parameter_size(raw: &str) -> Option<u64> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let (digits, multiplier) = match trimmed.chars().last()?.to_ascii_uppercase() {
        'B' => (&trimmed[..trimmed.len() - 1], 1_000_000_000.0),
        'M' => (&trimmed[..trimmed.len() - 1], 1_000_000.0),
        'K' => (&trimmed[..trimmed.len() - 1], 1_000.0),
        _ => (trimmed, 1.0),
    };
    let value: f64 = digits.trim().parse().ok()?;
    if value <= 0.0 {
        return None;
    }
    Some((value * multiplier) as u64)
}

fn entry_params(entry: &serde_json::Value) -> Option<u64> {
    // llama.cpp reports an exact count under meta.n_params.
    if let Some(count) = entry
        .get("meta")
        .and_then(|meta| meta.get("n_params"))
        .and_then(serde_json::Value::as_u64)
    {
        return Some(count);
    }
    // Ollama reports a human string under details.parameter_size.
    entry
        .get("details")
        .and_then(|details| details.get("parameter_size"))
        .and_then(serde_json::Value::as_str)
        .and_then(parse_parameter_size)
}

fn entry_capabilities(entry: &serde_json::Value) -> Vec<String> {
    entry
        .get("capabilities")
        .and_then(serde_json::Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn entry_context_tokens(entry: &serde_json::Value) -> Option<u32> {
    entry
        .get("meta")
        .and_then(|meta| meta.get("n_ctx_train"))
        .and_then(serde_json::Value::as_u64)
        .or_else(|| {
            entry
                .get("details")
                .and_then(|details| details.get("context_length"))
                .and_then(serde_json::Value::as_u64)
        })
        .or_else(|| {
            entry
                .get("context_length")
                .and_then(serde_json::Value::as_u64)
        })
        .and_then(|value| u32::try_from(value).ok())
}

fn entry_bytes(entry: &serde_json::Value) -> Option<u64> {
    entry
        .get("meta")
        .and_then(|meta| meta.get("size"))
        .and_then(serde_json::Value::as_u64)
        .or_else(|| entry.get("size").and_then(serde_json::Value::as_u64))
}

/// Reads model identifiers and size signals out of either the OpenAI
/// (`data[].id`) or the Ollama (`models[].name`) listing shape.
pub fn parse_models(body: &serde_json::Value) -> Vec<ModelInfo> {
    let entries = body
        .get("data")
        .or_else(|| body.get("models"))
        .and_then(serde_json::Value::as_array);

    let Some(entries) = entries else {
        return Vec::new();
    };

    // llama.cpp answers with both arrays and splits the information between
    // them: `data[]` carries the size metadata, `models[]` carries the
    // capability list. Look up the sibling entry by name so neither is lost.
    let siblings = body
        .get("models")
        .and_then(serde_json::Value::as_array)
        .filter(|_| body.get("data").is_some());
    let sibling_for = |name: &str| -> Option<&serde_json::Value> {
        siblings?.iter().find(|entry| {
            entry
                .get("name")
                .or_else(|| entry.get("model"))
                .and_then(serde_json::Value::as_str)
                .map(|candidate| candidate.trim() == name)
                .unwrap_or(false)
        })
    };

    let mut models: Vec<ModelInfo> = Vec::new();
    for entry in entries {
        let name = entry
            .get("id")
            .or_else(|| entry.get("name"))
            .or_else(|| entry.get("model"))
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());

        if let Some(name) = name {
            if models.iter().any(|existing| existing.name == name) {
                continue;
            }
            let sibling = sibling_for(name);
            let mut capabilities = entry_capabilities(entry);
            if capabilities.is_empty() {
                if let Some(sibling) = sibling {
                    capabilities = entry_capabilities(sibling);
                }
            }
            models.push(ModelInfo {
                name: name.to_string(),
                params: entry_params(entry).or_else(|| sibling.and_then(entry_params)),
                bytes: entry_bytes(entry).or_else(|| sibling.and_then(entry_bytes)),
                capabilities,
                context_tokens: entry_context_tokens(entry)
                    .or_else(|| sibling.and_then(entry_context_tokens)),
            });
        }
    }

    models
}

/// Reads the served context window from an OpenAI-style model listing.
///
/// vLLM publishes it as `max_model_len` on the model entry, where llama.cpp
/// uses `/props`. Either way it is the window the endpoint will actually honour.
pub fn parse_listing_context(body: &serde_json::Value) -> Option<u32> {
    body.get("data")
        .and_then(serde_json::Value::as_array)?
        .iter()
        .filter_map(|entry| {
            entry
                .get("max_model_len")
                .and_then(serde_json::Value::as_u64)
        })
        .max()
        .filter(|value| *value > 0)
        .and_then(|value| u32::try_from(value).ok())
}

/// Reads the context window a llama.cpp-style server is actually serving from
/// its `/props` endpoint.
pub fn parse_served_context(body: &serde_json::Value) -> Option<u32> {
    body.get("default_generation_settings")
        .and_then(|settings| settings.get("n_ctx"))
        .or_else(|| body.get("n_ctx"))
        .and_then(serde_json::Value::as_u64)
        .filter(|value| *value > 0)
        .and_then(|value| u32::try_from(value).ok())
}

/// Server-reported capacity, read from a Prometheus `/metrics` body.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ServerCapacity {
    pub memory_utilization: Option<f32>,
    pub max_concurrency: Option<u32>,
    pub kv_cache_tokens: Option<u64>,
    pub supports_tool_calls: bool,
}

fn metrics_label(body: &str, label: &str) -> Option<String> {
    // Labels appear as `name="value"` inside `metric{...}` lines.
    let needle = format!("{label}=\"");
    let start = body.find(&needle)? + needle.len();
    let rest = &body[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// Reads capacity from a vLLM-style `/metrics` body.
///
/// `vllm:cache_config_info` carries the engine's resolved configuration as
/// labels, which is the only place the runtime publishes what share of the
/// machine it took and how many sequences it can hold.
pub fn parse_server_capacity(body: &str) -> ServerCapacity {
    ServerCapacity {
        memory_utilization: metrics_label(body, "gpu_memory_utilization")
            .and_then(|value| value.parse::<f32>().ok())
            .filter(|value| *value > 0.0 && *value <= 1.0),
        max_concurrency: metrics_label(body, "kv_cache_max_concurrency")
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|value| *value >= 1.0)
            .map(|value| value as u32),
        kv_cache_tokens: metrics_label(body, "kv_cache_size_tokens")
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| *value > 0),
        // The parser metric only exists when a tool-call parser is loaded, so
        // its presence is the runtime telling us tool calling is configured.
        supports_tool_calls: body.contains("tool_call_parser_invocations"),
    }
}

/// Reads vLLM's configured scheduler ceiling from `/server_info` without
/// retaining the rest of the diagnostic response. Newer servers return a
/// nested JSON config; older versions place the config in a debug string.
pub fn parse_max_num_seqs(body: &serde_json::Value) -> Option<u32> {
    fn visit(value: &serde_json::Value) -> Option<u32> {
        match value {
            serde_json::Value::Object(entries) => {
                if let Some(limit) = entries
                    .get("max_num_seqs")
                    .and_then(serde_json::Value::as_u64)
                    .filter(|limit| *limit > 0)
                    .and_then(|limit| u32::try_from(limit).ok())
                {
                    return Some(limit);
                }
                entries.values().find_map(visit)
            }
            serde_json::Value::Array(entries) => entries.iter().find_map(visit),
            serde_json::Value::String(raw) => parse_named_positive_u32(raw, "max_num_seqs"),
            _ => None,
        }
    }

    visit(body)
}

fn parse_named_positive_u32(raw: &str, name: &str) -> Option<u32> {
    let start = raw.find(name)? + name.len();
    let separator = raw[start..].find(|character| matches!(character, ':' | '='))? + start + 1;
    let value = raw[separator..].trim_start();
    if value.starts_with('-') {
        return None;
    }
    let digits: String = value.chars().take_while(char::is_ascii_digit).collect();
    digits.parse::<u32>().ok().filter(|value| *value > 0)
}

/// Identifies the runtime from its own model listing rather than from the port
/// it happens to occupy. A llama.cpp server on `8000` is llama.cpp, not vLLM.
pub fn identify_kind(body: &serde_json::Value, base_url: &str) -> ClusterKind {
    let entries = body
        .get("data")
        .and_then(serde_json::Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);

    for entry in entries {
        let owned_by = entry
            .get("owned_by")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_ascii_lowercase();
        if owned_by.contains("llamacpp") || owned_by.contains("llama.cpp") {
            return ClusterKind::LlamaCpp;
        }
        if owned_by.contains("vllm") {
            return ClusterKind::Vllm;
        }
    }

    // `meta.n_ctx_train` / `meta.n_vocab` are llama.cpp-specific model metadata.
    if entries.iter().any(|entry| {
        entry
            .get("meta")
            .map(|meta| meta.get("n_ctx_train").is_some() || meta.get("n_vocab").is_some())
            .unwrap_or(false)
    }) {
        return ClusterKind::LlamaCpp;
    }

    // Ollama's /api/tags has no `data` array and carries per-model `details`.
    if entries.is_empty() {
        if let Some(models) = body.get("models").and_then(serde_json::Value::as_array) {
            if models
                .iter()
                .any(|entry| entry.get("details").is_some() || entry.get("digest").is_some())
            {
                return ClusterKind::Ollama;
            }
        }
    }

    ClusterKind::probe_hint(base_url)
}

/// Detected clusters ordered by the size of the biggest model each one serves,
/// largest first. Endpoints with nothing loaded are dropped: they cannot serve
/// work, so offering them would only produce a node that is never ready.
pub fn servable_clusters_by_size(clusters: &[DetectedCluster]) -> Vec<&DetectedCluster> {
    let mut ranked: Vec<&DetectedCluster> = clusters
        .iter()
        .filter(|cluster| cluster.is_servable())
        .collect();
    ranked.sort_by_key(|cluster| std::cmp::Reverse(cluster.size_key()));
    ranked
}

/// The endpoint to offer by default: the biggest servable one, falling back to
/// the first detected endpoint so an idle runtime can still be reported.
pub fn preferred_cluster(clusters: &[DetectedCluster]) -> Option<&DetectedCluster> {
    servable_clusters_by_size(clusters)
        .first()
        .copied()
        .or_else(|| clusters.first())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fetcher(
        responses: Vec<(&'static str, serde_json::Value)>,
    ) -> impl Fn(&str) -> Option<serde_json::Value> {
        move |url: &str| {
            responses
                .iter()
                .find(|(candidate, _)| *candidate == url)
                .map(|(_, body)| body.clone())
        }
    }

    fn names(models: &[ModelInfo]) -> Vec<&str> {
        models.iter().map(|model| model.name.as_str()).collect()
    }

    fn cluster(kind: ClusterKind, base_url: &str, models: Vec<ModelInfo>) -> DetectedCluster {
        DetectedCluster {
            kind,
            base_url: base_url.to_string(),
            models,
            served_context_tokens: None,
            memory_utilization: None,
            max_concurrency: None,
            max_num_seqs: None,
            kv_cache_tokens: None,
            supports_tool_calls: false,
        }
    }

    #[test]
    fn parses_openai_model_listing() {
        let body = json!({"data": [{"id": "qwen2.5-7b"}, {"id": "llama-3.1-8b"}]});

        assert_eq!(
            names(&parse_models(&body)),
            vec!["qwen2.5-7b", "llama-3.1-8b"]
        );
    }

    #[test]
    fn parses_ollama_tag_listing() {
        let body = json!({"models": [{"name": "llama3.1:8b"}, {"name": "mistral:7b"}]});

        assert_eq!(
            names(&parse_models(&body)),
            vec!["llama3.1:8b", "mistral:7b"]
        );
    }

    #[test]
    fn parses_empty_listing_without_models() {
        assert!(parse_models(&json!({"data": []})).is_empty());
        assert!(parse_models(&json!({"error": "unauthorized"})).is_empty());
    }

    #[test]
    fn deduplicates_repeated_model_identifiers() {
        let body = json!({"data": [{"id": "qwen"}, {"id": "qwen"}, {"id": " "}]});

        assert_eq!(names(&parse_models(&body)), vec!["qwen"]);
    }

    #[test]
    fn reads_exact_parameter_counts_from_a_llama_cpp_listing() {
        // Shape returned by the llama.cpp server's /v1/models.
        let body = json!({
            "data": [{
                "id": "UD-IQ2_M",
                "meta": {"n_params": 753_864_139_008u64, "size": 238_568_039_424u64},
            }]
        });

        let models = parse_models(&body);

        assert_eq!(models[0].params, Some(753_864_139_008));
        assert_eq!(models[0].bytes, Some(238_568_039_424));
    }

    #[test]
    fn reads_parameter_sizes_and_bytes_from_an_ollama_listing() {
        let body = json!({
            "models": [{
                "name": "hermes3:70b",
                "size": 39_969_747_239u64,
                "details": {"parameter_size": "70.6B"},
            }]
        });

        let models = parse_models(&body);

        assert_eq!(models[0].params, Some(70_600_000_000));
        assert_eq!(models[0].bytes, Some(39_969_747_239));
    }

    #[test]
    fn parses_human_parameter_size_strings() {
        assert_eq!(parse_parameter_size("8.0B"), Some(8_000_000_000));
        assert_eq!(parse_parameter_size("70b"), Some(70_000_000_000));
        assert_eq!(parse_parameter_size("350M"), Some(350_000_000));
        assert_eq!(parse_parameter_size(""), None);
        assert_eq!(parse_parameter_size("unknown"), None);
        assert_eq!(parse_parameter_size("0B"), None);
    }

    #[test]
    fn a_cluster_advertises_its_biggest_model_not_its_first() {
        let ollama = cluster(
            ClusterKind::Ollama,
            "http://127.0.0.1:11434",
            vec![
                ModelInfo::new("hermes3:8b", Some(8_000_000_000), None),
                ModelInfo::new("hermes3:70b", Some(70_000_000_000), None),
            ],
        );

        assert_eq!(ollama.primary_model(), Some("hermes3:70b"));
    }

    #[test]
    fn the_biggest_cluster_wins_over_probe_order() {
        // Ollama is probed first, but llama.cpp serves the far larger model.
        let clusters = vec![
            cluster(
                ClusterKind::Ollama,
                "http://127.0.0.1:11434",
                vec![ModelInfo::new("hermes3:8b", Some(8_000_000_000), None)],
            ),
            cluster(
                ClusterKind::Vllm,
                "http://127.0.0.1:8000",
                vec![ModelInfo::new("UD-IQ2_M", Some(753_864_139_008), None)],
            ),
        ];

        assert_eq!(
            preferred_cluster(&clusters).map(|entry| entry.base_url.as_str()),
            Some("http://127.0.0.1:8000")
        );

        let ranked = servable_clusters_by_size(&clusters);
        assert_eq!(ranked[0].base_url, "http://127.0.0.1:8000");
        assert_eq!(ranked[1].base_url, "http://127.0.0.1:11434");
    }

    #[test]
    fn on_disk_bytes_break_ties_when_no_parameter_count_is_reported() {
        let clusters = vec![
            cluster(
                ClusterKind::LmStudio,
                "http://127.0.0.1:1234",
                vec![ModelInfo::new("small", None, Some(1_000))],
            ),
            cluster(
                ClusterKind::OpenAiCompatible,
                "http://127.0.0.1:8080",
                vec![ModelInfo::new("large", None, Some(9_000))],
            ),
        ];

        assert_eq!(
            preferred_cluster(&clusters).unwrap().base_url,
            "http://127.0.0.1:8080"
        );
    }

    #[test]
    fn idle_endpoints_are_ranked_out_but_still_reportable() {
        let idle = cluster(ClusterKind::LmStudio, "http://127.0.0.1:1234", Vec::new());
        let servable = cluster(
            ClusterKind::Ollama,
            "http://127.0.0.1:11434",
            vec![ModelInfo::new("hermes3:8b", Some(8_000_000_000), None)],
        );

        assert!(servable_clusters_by_size(&[idle.clone()]).is_empty());
        assert_eq!(preferred_cluster(&[idle.clone()]), Some(&idle));
        assert_eq!(
            preferred_cluster(&[idle, servable.clone()]),
            Some(&servable)
        );
    }

    #[test]
    fn identifies_llama_cpp_from_its_own_listing_not_its_port() {
        // Real shape from a llama.cpp server sitting on vLLM's default port.
        let body = json!({
            "object": "list",
            "data": [{
                "id": "UD-IQ2_M",
                "owned_by": "llamacpp",
                "meta": {"n_params": 753_864_139_008u64, "n_ctx_train": 1_048_576},
            }]
        });

        assert_eq!(
            identify_kind(&body, "http://127.0.0.1:8000"),
            ClusterKind::LlamaCpp
        );
    }

    #[test]
    fn identifies_llama_cpp_from_model_metadata_without_owned_by() {
        let body = json!({"data": [{"id": "m", "meta": {"n_vocab": 154_880}}]});

        assert_eq!(
            identify_kind(&body, "http://127.0.0.1:8000"),
            ClusterKind::LlamaCpp
        );
    }

    #[test]
    fn reads_server_capacity_from_a_vllm_metrics_body() {
        // Real shape: the engine publishes its resolved config as labels.
        let body = concat!(
            "# HELP vllm:cache_config_info Cache config\n",
            "vllm:cache_config_info{cache_dtype=\"fp8\",engine=\"0\",",
            "gpu_memory_utilization=\"0.85\",kv_cache_max_concurrency=\"71.98449612403101\",",
            "kv_cache_size_tokens=\"9435151\",num_gpu_blocks=\"9286\"} 1.0\n",
            "vllm:tool_call_parser_invocations_total{model_name=\"qwen3-coder\"} 0.0\n",
        );

        let cap = parse_server_capacity(body);

        assert_eq!(cap.memory_utilization, Some(0.85));
        assert_eq!(cap.max_concurrency, Some(71));
        assert_eq!(cap.kv_cache_tokens, Some(9_435_151));
        // The parser metric only exists when a tool-call parser is loaded.
        assert!(cap.supports_tool_calls);
    }

    #[test]
    fn a_runtime_without_a_tool_parser_does_not_claim_tools() {
        let body = "vllm:cache_config_info{gpu_memory_utilization=\"0.90\"} 1.0\n";

        let cap = parse_server_capacity(body);

        assert_eq!(cap.memory_utilization, Some(0.90));
        assert!(!cap.supports_tool_calls);
        assert_eq!(cap.max_concurrency, None);
    }

    #[test]
    fn nonsense_capacity_values_are_ignored() {
        let body = concat!(
            "vllm:cache_config_info{gpu_memory_utilization=\"0\",",
            "kv_cache_max_concurrency=\"0.4\",kv_cache_size_tokens=\"0\"} 1.0\n",
        );

        let cap = parse_server_capacity(body);

        assert_eq!(cap.memory_utilization, None); // 0 is not a share
        assert_eq!(cap.max_concurrency, None); // below one sequence
        assert_eq!(cap.kv_cache_tokens, None);
    }

    #[test]
    fn detection_records_server_capacity_alongside_the_listing() {
        let clusters = detect_with_text(
            vec!["http://127.0.0.1:8000".to_string()],
            fetcher(vec![
                (
                    "http://127.0.0.1:8000/v1/models",
                    json!({"data": [{"id": "qwen3-coder", "owned_by": "vllm"}]}),
                ),
                (
                    "http://127.0.0.1:8000/server_info?config_format=json",
                    json!({"vllm_config": {"scheduler_config": {"max_num_seqs": 32}}}),
                ),
            ]),
            |url| {
                if url.ends_with("/metrics") {
                    Some(
                        concat!(
                            "vllm:cache_config_info{gpu_memory_utilization=\"0.85\",",
                            "kv_cache_max_concurrency=\"71.9\"} 1.0\n",
                            "vllm:tool_call_parser_invocations_total{} 0.0\n",
                        )
                        .to_string(),
                    )
                } else {
                    None
                }
            },
        );

        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].memory_utilization, Some(0.85));
        assert_eq!(clusters[0].max_concurrency, Some(71));
        assert_eq!(clusters[0].max_num_seqs, Some(32));
        assert!(clusters[0].supports_tool_calls);
    }

    #[test]
    fn reads_max_num_seqs_from_structured_and_legacy_server_info() {
        assert_eq!(
            parse_max_num_seqs(&json!({
                "vllm_config": {"scheduler_config": {"max_num_seqs": 32}}
            })),
            Some(32)
        );
        assert_eq!(
            parse_max_num_seqs(&json!({
                "vllm_config": "SchedulerConfig(max_num_seqs=20, max_num_batched_tokens=8192)"
            })),
            Some(20)
        );
        assert_eq!(parse_max_num_seqs(&json!({"max_num_seqs": 0})), None);
    }

    #[test]
    fn reads_the_context_window_the_server_actually_serves() {
        // A llama.cpp server started with `-c 256` serves 256 tokens even when
        // the GGUF was trained at 1M, so /props is the authority.
        let props = json!({"default_generation_settings": {"n_ctx": 256}, "total_slots": 1});

        assert_eq!(parse_served_context(&props), Some(256));
        assert_eq!(parse_served_context(&json!({"n_ctx": 8192})), Some(8192));
        assert_eq!(parse_served_context(&json!({"n_ctx": 0})), None);
        assert_eq!(parse_served_context(&json!({})), None);
    }

    #[test]
    fn reads_the_served_window_from_a_vllm_listing() {
        // vLLM reports the served window on the model entry, not via /props.
        let body =
            json!({"data": [{"id": "qwen3-coder", "owned_by": "vllm", "max_model_len": 131072}]});

        assert_eq!(parse_listing_context(&body), Some(131_072));
        assert_eq!(parse_listing_context(&json!({"data": [{"id": "m"}]})), None);
    }

    #[test]
    fn props_wins_over_the_listing_when_both_are_present() {
        // llama.cpp can be started with a smaller -c than the model allows, and
        // /props is what it will actually honour.
        let clusters = detect_with_text(
            vec!["http://127.0.0.1:8000".to_string()],
            fetcher(vec![
                (
                    "http://127.0.0.1:8000/v1/models",
                    json!({"data": [{"id": "m", "owned_by": "llamacpp", "max_model_len": 131072}]}),
                ),
                (
                    "http://127.0.0.1:8000/props",
                    json!({"default_generation_settings": {"n_ctx": 4096}}),
                ),
            ]),
            |_| None,
        );

        assert_eq!(clusters[0].served_context_tokens, Some(4096));
    }

    #[test]
    fn detection_records_the_served_context() {
        let clusters = detect_with(
            vec!["http://127.0.0.1:8000".to_string()],
            fetcher(vec![
                (
                    "http://127.0.0.1:8000/v1/models",
                    json!({"data": [{
                        "id": "UD-IQ2_M",
                        "owned_by": "llamacpp",
                        "meta": {"n_params": 753_864_139_008u64, "n_ctx_train": 1_048_576},
                    }]}),
                ),
                (
                    "http://127.0.0.1:8000/props",
                    json!({"default_generation_settings": {"n_ctx": 256}}),
                ),
            ]),
        );

        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].kind, ClusterKind::LlamaCpp);
        // Trained context is still parsed from the listing...
        assert_eq!(clusters[0].models[0].context_tokens, Some(1_048_576));
        // ...but the served window is what the node will advertise.
        assert_eq!(clusters[0].served_context_tokens, Some(256));
    }

    #[test]
    fn merges_llama_cpp_split_listing_arrays() {
        // llama.cpp answers with both arrays and splits the information:
        // sizes and context live in `data[]`, capabilities in `models[]`.
        let body = json!({
            "models": [{"name": "UD-IQ2_M", "capabilities": ["completion"]}],
            "object": "list",
            "data": [{
                "id": "UD-IQ2_M",
                "owned_by": "llamacpp",
                "meta": {
                    "n_params": 753_864_139_008u64,
                    "size": 238_568_039_424u64,
                    "n_ctx_train": 1_048_576,
                },
            }]
        });

        let models = parse_models(&body);

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].params, Some(753_864_139_008));
        assert_eq!(models[0].context_tokens, Some(1_048_576));
        assert_eq!(models[0].capabilities, vec!["completion"]);
        assert!(!models[0].supports_tools());
    }

    #[test]
    fn reads_capabilities_and_context_from_an_ollama_listing() {
        let body = json!({
            "models": [{
                "name": "hermes3:70b",
                "capabilities": ["completion", "tools"],
                "details": {"parameter_size": "70.6B", "context_length": 131_072},
            }]
        });

        let models = parse_models(&body);

        assert!(models[0].supports_tools());
        assert_eq!(models[0].context_tokens, Some(131_072));
    }

    #[test]
    fn recognizes_embedding_and_vision_capabilities() {
        let embed = json!({"data": [{"id": "nomic-embed-text", "capabilities": ["embedding"]}]});
        assert!(parse_models(&embed)[0].supports_embeddings());

        // Name is a fallback when the runtime reports nothing.
        let unnamed = json!({"data": [{"id": "text-embedding-3-small"}]});
        assert!(parse_models(&unnamed)[0].supports_embeddings());

        let vision = json!({"data": [{"id": "llava", "capabilities": ["vision"]}]});
        assert!(parse_models(&vision)[0].supports_vision());
    }

    #[test]
    fn identifies_vllm_when_it_says_so() {
        let body = json!({"data": [{"id": "m", "owned_by": "vllm"}]});

        assert_eq!(
            identify_kind(&body, "http://127.0.0.1:9999"),
            ClusterKind::Vllm
        );
    }

    #[test]
    fn identifies_ollama_from_its_tag_listing() {
        let body = json!({"models": [{"name": "hermes3:8b", "digest": "abc", "details": {}}]});

        assert_eq!(
            identify_kind(&body, "http://127.0.0.1:9999"),
            ClusterKind::Ollama
        );
    }

    #[test]
    fn falls_back_to_the_port_hint_when_the_listing_says_nothing() {
        let plain = json!({"data": [{"id": "m", "object": "model"}]});

        assert_eq!(
            identify_kind(&plain, "http://127.0.0.1:1234"),
            ClusterKind::LmStudio
        );
        assert_eq!(
            identify_kind(&plain, "http://127.0.0.1:4000"),
            ClusterKind::OpenAiCompatible
        );
    }

    #[test]
    fn detects_ollama_on_its_native_path() {
        let clusters = detect_with(
            vec!["http://127.0.0.1:11434".to_string()],
            fetcher(vec![(
                "http://127.0.0.1:11434/api/tags",
                json!({"models": [{"name": "llama3.1:8b"}]}),
            )]),
        );

        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].kind, ClusterKind::Ollama);
        assert_eq!(clusters[0].primary_model(), Some("llama3.1:8b"));
    }

    #[test]
    fn falls_back_to_the_openai_path_when_the_native_one_is_silent() {
        let clusters = detect_with(
            vec!["http://127.0.0.1:11434".to_string()],
            fetcher(vec![(
                "http://127.0.0.1:11434/v1/models",
                json!({"data": [{"id": "llama3.1:8b"}]}),
            )]),
        );

        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].primary_model(), Some("llama3.1:8b"));
    }

    #[test]
    fn ignores_the_mundusx_managed_runtime_port() {
        let clusters = detect_with(
            vec!["http://127.0.0.1:8789".to_string()],
            fetcher(vec![(
                "http://127.0.0.1:8789/v1/models",
                json!({"data": [{"id": "ours"}]}),
            )]),
        );

        assert!(clusters.is_empty());
    }

    #[test]
    fn reports_a_running_endpoint_with_no_loaded_model_as_not_servable() {
        let clusters = detect_with(
            vec!["http://127.0.0.1:1234".to_string()],
            fetcher(vec![(
                "http://127.0.0.1:1234/v1/models",
                json!({"data": []}),
            )]),
        );

        assert_eq!(clusters.len(), 1);
        assert!(!clusters[0].is_servable());
    }

    #[test]
    fn asks_the_contributor_when_a_servable_cluster_is_new() {
        assert_eq!(
            cluster_prompt_decision(true, false, false, None, true),
            ClusterPromptDecision::Ask
        );
    }

    #[test]
    fn flags_answer_for_the_contributor() {
        assert_eq!(
            cluster_prompt_decision(true, false, false, Some(true), false),
            ClusterPromptDecision::AutoContribute
        );
        assert_eq!(
            cluster_prompt_decision(true, false, false, Some(false), true),
            ClusterPromptDecision::AutoDecline
        );
    }

    #[test]
    fn forced_contribution_re_adopts_an_already_contributed_cluster() {
        assert_eq!(
            cluster_prompt_decision(true, true, true, Some(true), false),
            ClusterPromptDecision::AutoContribute
        );
    }

    #[test]
    fn does_not_re_ask_after_a_decline_or_an_adoption() {
        assert!(matches!(
            cluster_prompt_decision(true, true, false, None, true),
            ClusterPromptDecision::Skip(_)
        ));
        assert!(matches!(
            cluster_prompt_decision(true, false, true, None, true),
            ClusterPromptDecision::Skip(_)
        ));
    }

    #[test]
    fn never_prompts_without_a_terminal() {
        assert!(matches!(
            cluster_prompt_decision(true, false, false, None, false),
            ClusterPromptDecision::Skip(_)
        ));
    }

    #[test]
    fn skips_everything_when_nothing_is_running() {
        assert!(matches!(
            cluster_prompt_decision(false, false, false, Some(true), true),
            ClusterPromptDecision::Skip(_)
        ));
    }

    #[test]
    fn normalizes_only_http_base_urls() {
        assert_eq!(
            normalize_base_url(" http://127.0.0.1:8000/ "),
            Some("http://127.0.0.1:8000".to_string())
        );
        assert_eq!(normalize_base_url("127.0.0.1:8000"), None);
    }

    #[test]
    fn formats_sizes_for_menus() {
        assert_eq!(format_params(753_864_139_008), "754B params");
        assert_eq!(format_params(8_000_000_000), "8.0B params");
        assert_eq!(format_params(350_000_000), "350M params");
        assert_eq!(format_bytes(238_568_039_424), "222.2 GB");
    }

    #[test]
    fn summarizes_a_detected_cluster_biggest_first() {
        let entry = cluster(
            ClusterKind::Ollama,
            "http://127.0.0.1:11434",
            vec![
                ModelInfo::new("small", Some(1_000_000_000), None),
                ModelInfo::new("big", Some(70_000_000_000), None),
                ModelInfo::new("mid", Some(8_000_000_000), None),
                ModelInfo::new("tiny", Some(500_000_000), None),
            ],
        );

        assert_eq!(
            entry.summary(),
            "Ollama at http://127.0.0.1:11434 (4 models: big (70.0B params), mid (8.0B params), small (1.0B params) +1 more)"
        );
    }
}
