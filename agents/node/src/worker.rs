use crate::contracts::{
    Backend, ModelCapability, WorkerHealthReport, WorkerLaunchRequest, WorkerLaunchResponse,
    WorkerPolicyReport,
};
use clap::Parser;
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::cell::RefCell;
use std::env;
use std::fs;
use std::fs::OpenOptions;
use std::io;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

const STREAM_DELTA_PREFIX: &str = "MUNDUSX_STREAM_DELTA:";
const STREAM_TAIL_HOLD_CHARS: usize = 32;

thread_local! {
    static STREAM_DELTA_SENDER: RefCell<Option<mpsc::Sender<String>>> = const { RefCell::new(None) };
}

fn with_stream_delta_sender<T>(
    sender: Option<mpsc::Sender<String>>,
    action: impl FnOnce() -> T,
) -> T {
    STREAM_DELTA_SENDER.with(|slot| {
        let previous = slot.replace(sender);
        let result = action();
        slot.replace(previous);
        result
    })
}

fn live_delta_enabled() -> bool {
    STREAM_DELTA_SENDER.with(|slot| slot.borrow().is_some())
}

fn emit_stream_delta(delta: &str) {
    if delta.is_empty() {
        return;
    }
    STREAM_DELTA_SENDER.with(|slot| {
        if let Some(sender) = slot.borrow().as_ref() {
            let _ = sender.send(delta.to_string());
        }
    });
}

#[derive(Debug, Default, Clone, Serialize)]
struct RuntimeMetrics {
    #[serde(skip_serializing_if = "Option::is_none")]
    total_duration_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    load_duration_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    prompt_eval_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    prompt_eval_duration_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    prompt_eval_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    eval_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    eval_duration_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    eval_rate: Option<f64>,
}

impl RuntimeMetrics {
    fn is_empty(&self) -> bool {
        self.total_duration_ms.is_none()
            && self.load_duration_ms.is_none()
            && self.prompt_eval_count.is_none()
            && self.prompt_eval_duration_ms.is_none()
            && self.prompt_eval_rate.is_none()
            && self.eval_count.is_none()
            && self.eval_duration_ms.is_none()
            && self.eval_rate.is_none()
    }

    fn to_output_fragment(&self) -> Option<String> {
        if self.is_empty() {
            return None;
        }
        serde_json::to_string(self)
            .ok()
            .map(|json| format!("; runtime_metrics={json}"))
    }
}

#[derive(Parser, Debug)]
#[command(
    name = "opengpu-agent worker",
    version,
    about = "MundusX local worker process",
    arg_required_else_help = true
)]
pub struct WorkerCli {
    #[arg(long)]
    pub job_id: String,
    #[arg(long)]
    pub node_id: String,
    #[arg(long)]
    pub prompt: String,
    #[arg(long)]
    pub model: Option<String>,
    #[arg(long)]
    pub mode: Option<String>,
    #[arg(long)]
    pub system_prompt: Option<String>,
    #[arg(long)]
    pub max_tokens: Option<u32>,
    #[arg(long)]
    pub temperature: Option<f32>,
    #[arg(long)]
    pub top_p: Option<f32>,
    #[arg(long)]
    pub seed: Option<u64>,
    #[arg(long)]
    pub backend: Backend,
    #[arg(long)]
    pub json: bool,
    #[arg(long)]
    pub stream: bool,
}

fn emit_json<T: Serialize>(value: &T) -> Result<(), String> {
    let payload = serde_json::to_string_pretty(value).map_err(|error| error.to_string())?;
    println!("{payload}");
    Ok(())
}

fn resolved_backend(backend: Backend) -> Backend {
    match backend {
        Backend::Auto => {
            #[cfg(target_os = "macos")]
            {
                if std::env::consts::ARCH == "aarch64" {
                    return Backend::M;
                }
            }

            Backend::Auto
        }
        other => other,
    }
}

fn now_unix_seconds() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs().to_string())
        .unwrap_or_else(|_| "0".to_string())
}

#[derive(Debug, Clone)]
struct PowerState {
    source: String,
    on_battery: bool,
    battery_percent: Option<u8>,
}

#[derive(Debug, Clone, Default)]
pub struct CudaDiagnostics {
    pub device_available: bool,
    pub driver_available: bool,
    pub device_name: Option<String>,
    pub memory_mb: Option<u32>,
    pub low_vram_profile: bool,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TrustedExecutable {
    pub path: String,
    #[serde(default)]
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct TrustedRuntimePaths {
    #[serde(default)]
    pub llama_cli: Option<TrustedExecutable>,
    #[serde(default)]
    pub llama_server: Option<TrustedExecutable>,
    #[serde(default)]
    pub nvidia_smi: Option<TrustedExecutable>,
}

#[derive(Debug, Clone, Deserialize)]
struct CachedModelRecord {
    name: String,
    active: bool,
    #[serde(default)]
    source_path: Option<String>,
    #[serde(default)]
    file_name: Option<String>,
    #[serde(default)]
    format: Option<String>,
    #[serde(default)]
    quantization: Option<String>,
    #[serde(default)]
    size_bytes: Option<u64>,
    #[serde(default)]
    estimated_vram_mb: Option<u64>,
    #[serde(default)]
    compatibility: Option<String>,
    #[serde(default)]
    compatibility_reason: Option<String>,
    #[serde(default)]
    specialties: Vec<String>,
    #[serde(default)]
    languages: Vec<String>,
    #[serde(default)]
    supports_structured_output: bool,
}

const SPEAKAI_MAX_ATTEMPTS: u32 = 3;

const SPEAKAI_SYSTEM_PROMPT: &str = r#"You are SpeakAI, a conversation-learning response generator.
Analyze the user's utterance and return only one complete JSON object. Do not use Markdown or commentary.
Use speechAct: opinion, question, observation, request, invitation, suggestion, greeting, thanks, apology, compliment, emotion, or information.
Always include a non-empty English topic and exactly three replies. summary must be only a direct, natural English translation of the user's utterance, never an explanation such as "The speaker asks...". Each reply must contain strategy, purpose, text, and meaning. Reply text must use the utterance language; meaning must be a natural English translation.
For questions only, include questionType: factual, personal, opinion, clarification, preference, hypothetical, or other.
Use these strategy/purpose pairs in order:
opinion: supportive/AGREE, continue/EXPLORE, alternative/DISAGREE_POLITELY
question: direct/ANSWER, continue/ANSWER_AND_EXPLORE, boundary/DECLINE_POLITELY
observation: acknowledge/ACKNOWLEDGE, continue/EXPLORE, alternative/OFFER_ALTERNATIVE
request: accept/ACCEPT, clarify/CLARIFY, boundary/DECLINE_POLITELY
invitation: accept/ACCEPT, clarify/ASK_DETAILS, boundary/DECLINE_POLITELY
suggestion: supportive/SUPPORT, continue/EXPLORE, alternative/SUGGEST_ALTERNATIVE
greeting: direct/RETURN_GREETING, continue/START_CONVERSATION, warm/WARM_VARIATION
thanks: direct/ACCEPT_THANKS, warm/RESPOND_WARMLY, continue/CONTINUE
apology: accept/ACCEPT_APOLOGY, reassure/REASSURE, continue/DISCUSS_FURTHER
compliment: accept/ACCEPT_COMPLIMENT, reciprocal/RECIPROCATE, modest/RESPOND_MODESTLY
emotion: empathetic/EMPATHIZE, continue/EXPLORE, supportive/OFFER_SUPPORT
information: acknowledge/ACKNOWLEDGE, continue/ASK_FOLLOW_UP, related/ADD_RELATED_POINT"#;

const SPEAKAI_JSON_GRAMMAR: &str = r#"root ::= object
object ::= "{" ws speech-act "," ws topic "," ws summary question-type? "," ws replies ws "}"
speech-act ::= "\"speechAct\"" ws ":" ws ("\"opinion\"" | "\"question\"" | "\"observation\"" | "\"request\"" | "\"invitation\"" | "\"suggestion\"" | "\"greeting\"" | "\"thanks\"" | "\"apology\"" | "\"compliment\"" | "\"emotion\"" | "\"information\"")
topic ::= "\"topic\"" ws ":" ws string
summary ::= "\"summary\"" ws ":" ws string
question-type ::= "," ws "\"questionType\"" ws ":" ws ("\"factual\"" | "\"personal\"" | "\"opinion\"" | "\"clarification\"" | "\"preference\"" | "\"hypothetical\"" | "\"other\"")
replies ::= "\"replies\"" ws ":" ws "[" ws reply "," ws reply "," ws reply ws "]"
reply ::= "{" ws "\"strategy\"" ws ":" ws string "," ws "\"purpose\"" ws ":" ws string "," ws "\"text\"" ws ":" ws string "," ws "\"meaning\"" ws ":" ws string ws "}"
string ::= "\"" chars "\""
chars ::= ([^"\\\x7F\x00-\x1F] | "\\" (["\\/bfnrt] | "u" [0-9a-fA-F] [0-9a-fA-F] [0-9a-fA-F] [0-9a-fA-F]))*
ws ::= [ \t\n\r]*
"#;

fn is_speakai_request(request: &WorkerLaunchRequest) -> bool {
    request
        .mode
        .as_deref()
        .is_some_and(|mode| mode.eq_ignore_ascii_case("speakai"))
}

fn effective_system_prompt(request: &WorkerLaunchRequest) -> &str {
    if is_speakai_request(request) {
        SPEAKAI_SYSTEM_PROMPT
    } else {
        request.system_prompt.as_deref().unwrap_or("").trim()
    }
}

fn speakai_json_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["speechAct", "topic", "summary", "replies"],
        "properties": {
            "speechAct": {
                "type": "string",
                "enum": ["opinion", "question", "observation", "request", "invitation", "suggestion", "greeting", "thanks", "apology", "compliment", "emotion", "information"]
            },
            "questionType": {
                "type": "string",
                "enum": ["factual", "personal", "opinion", "clarification", "preference", "hypothetical", "other"]
            },
            "topic": { "type": "string", "minLength": 1 },
            "summary": { "type": "string", "minLength": 1 },
            "replies": {
                "type": "array",
                "minItems": 3,
                "maxItems": 3,
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["strategy", "purpose", "text", "meaning"],
                    "properties": {
                        "strategy": { "type": "string", "minLength": 1 },
                        "purpose": { "type": "string", "minLength": 1 },
                        "text": { "type": "string", "minLength": 1 },
                        "meaning": { "type": "string", "minLength": 1 }
                    }
                }
            }
        }
    })
}

fn speakai_response_format() -> serde_json::Value {
    serde_json::json!({
        "type": "json_schema",
        "json_schema": {
            "name": "speakai_response",
            "strict": true,
            "schema": speakai_json_schema()
        }
    })
}

fn speakai_reply_contract(speech_act: &str) -> Option<[(&'static str, &'static str); 3]> {
    Some(match speech_act {
        "opinion" => [
            ("supportive", "AGREE"),
            ("continue", "EXPLORE"),
            ("alternative", "DISAGREE_POLITELY"),
        ],
        "question" => [
            ("direct", "ANSWER"),
            ("continue", "ANSWER_AND_EXPLORE"),
            ("boundary", "DECLINE_POLITELY"),
        ],
        "observation" => [
            ("acknowledge", "ACKNOWLEDGE"),
            ("continue", "EXPLORE"),
            ("alternative", "OFFER_ALTERNATIVE"),
        ],
        "request" => [
            ("accept", "ACCEPT"),
            ("clarify", "CLARIFY"),
            ("boundary", "DECLINE_POLITELY"),
        ],
        "invitation" => [
            ("accept", "ACCEPT"),
            ("clarify", "ASK_DETAILS"),
            ("boundary", "DECLINE_POLITELY"),
        ],
        "suggestion" => [
            ("supportive", "SUPPORT"),
            ("continue", "EXPLORE"),
            ("alternative", "SUGGEST_ALTERNATIVE"),
        ],
        "greeting" => [
            ("direct", "RETURN_GREETING"),
            ("continue", "START_CONVERSATION"),
            ("warm", "WARM_VARIATION"),
        ],
        "thanks" => [
            ("direct", "ACCEPT_THANKS"),
            ("warm", "RESPOND_WARMLY"),
            ("continue", "CONTINUE"),
        ],
        "apology" => [
            ("accept", "ACCEPT_APOLOGY"),
            ("reassure", "REASSURE"),
            ("continue", "DISCUSS_FURTHER"),
        ],
        "compliment" => [
            ("accept", "ACCEPT_COMPLIMENT"),
            ("reciprocal", "RECIPROCATE"),
            ("modest", "RESPOND_MODESTLY"),
        ],
        "emotion" => [
            ("empathetic", "EMPATHIZE"),
            ("continue", "EXPLORE"),
            ("supportive", "OFFER_SUPPORT"),
        ],
        "information" => [
            ("acknowledge", "ACKNOWLEDGE"),
            ("continue", "ASK_FOLLOW_UP"),
            ("related", "ADD_RELATED_POINT"),
        ],
        _ => return None,
    })
}

fn required_json_string<'a>(value: &'a serde_json::Value, name: &str) -> Result<&'a str, String> {
    value
        .get(name)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|field| !field.is_empty())
        .ok_or_else(|| format!("SpeakAI output requires a non-empty `{name}`"))
}

fn extract_json_object(output: &str) -> Result<&str, String> {
    let response = output
        .split_once("; response=")
        .map(|(_, response)| response)
        .unwrap_or(output)
        .trim();
    let start = response
        .find('{')
        .ok_or_else(|| "SpeakAI output did not contain a JSON object".to_string())?;
    let end = response
        .rfind('}')
        .filter(|end| *end >= start)
        .ok_or_else(|| "SpeakAI output contained an incomplete JSON object".to_string())?;
    Ok(&response[start..=end])
}

fn validate_and_normalize_speakai_output(output: &str) -> Result<String, String> {
    let raw = extract_json_object(output)?;
    let value: serde_json::Value = serde_json::from_str(raw)
        .map_err(|error| format!("SpeakAI output was malformed JSON: {error}"))?;
    let object = value
        .as_object()
        .ok_or_else(|| "SpeakAI output must be a JSON object".to_string())?;
    let speech_act = required_json_string(&value, "speechAct")?.to_ascii_lowercase();
    let topic = required_json_string(&value, "topic")?;
    let summary = required_json_string(&value, "summary")?;
    let lower_summary = summary.to_ascii_lowercase();
    if [
        "the speaker ",
        "the user ",
        "the person ",
        "the utterance ",
    ]
    .iter()
    .any(|prefix| lower_summary.starts_with(prefix))
    {
        return Err(
            "SpeakAI summary must directly translate the utterance, not explain it".to_string(),
        );
    }
    let expected = speakai_reply_contract(&speech_act)
        .ok_or_else(|| format!("SpeakAI output used unsupported speechAct `{speech_act}`"))?;
    let replies = object
        .get("replies")
        .and_then(serde_json::Value::as_array)
        .filter(|replies| replies.len() == 3)
        .ok_or_else(|| "SpeakAI output requires exactly three replies".to_string())?;

    let mut normalized_replies = Vec::with_capacity(3);
    for (index, (reply, (strategy, purpose))) in replies.iter().zip(expected).enumerate() {
        let text = required_json_string(reply, "text")
            .map_err(|error| format!("reply {}: {error}", index + 1))?;
        let meaning = required_json_string(reply, "meaning")
            .map_err(|error| format!("reply {}: {error}", index + 1))?;
        normalized_replies.push(serde_json::json!({
            "strategy": strategy,
            "purpose": purpose,
            "text": text,
            "meaning": meaning,
        }));
    }

    let mut normalized = serde_json::Map::new();
    normalized.insert(
        "speechAct".to_string(),
        serde_json::Value::String(speech_act.clone()),
    );
    if speech_act == "question" {
        let question_type = object
            .get("questionType")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .map(str::to_ascii_lowercase)
            .filter(|value| {
                matches!(
                    value.as_str(),
                    "factual"
                        | "personal"
                        | "opinion"
                        | "clarification"
                        | "preference"
                        | "hypothetical"
                        | "other"
                )
            })
            .unwrap_or_else(|| "other".to_string());
        normalized.insert(
            "questionType".to_string(),
            serde_json::Value::String(question_type),
        );
    }
    normalized.insert(
        "topic".to_string(),
        serde_json::Value::String(topic.to_string()),
    );
    normalized.insert(
        "summary".to_string(),
        serde_json::Value::String(summary.to_string()),
    );
    normalized.insert(
        "replies".to_string(),
        serde_json::Value::Array(normalized_replies),
    );
    serde_json::to_string(&serde_json::Value::Object(normalized))
        .map_err(|error| format!("SpeakAI output normalization failed: {error}"))
}

fn speakai_retry_system_prompt(base: &str, attempt: u32, last_error: Option<&str>) -> String {
    if attempt == 0 {
        return base.to_string();
    }
    format!(
        "{base}\n\nYour previous SpeakAI response failed validation: {}. Return only one complete JSON object matching the required schema. Do not use Markdown fences or commentary.",
        last_error.unwrap_or("invalid structured output")
    )
}

fn normalize_generated_output(
    request: &WorkerLaunchRequest,
    generated: &str,
) -> Result<String, String> {
    if is_speakai_request(request) {
        validate_and_normalize_speakai_output(generated)
    } else {
        Ok(generated.to_string())
    }
}

fn sanitize_model_name(name: &str) -> String {
    let mut output = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            output.push(ch.to_ascii_lowercase());
        } else {
            output.push('_');
        }
    }

    let trimmed = output.trim_matches('_');
    if trimmed.is_empty() {
        "model".to_string()
    } else {
        trimmed.to_string()
    }
}

fn active_model_name_from_cache(model_dir: &Path) -> Option<String> {
    let manifest_dir = model_dir.join(".opengpu");
    let entries = fs::read_dir(manifest_dir).ok()?;
    for entry in entries.flatten() {
        if entry.file_type().ok()?.is_file()
            && entry.path().extension().and_then(|value| value.to_str()) == Some("json")
        {
            let raw = fs::read_to_string(entry.path()).ok()?;
            if let Ok(record) = serde_json::from_str::<CachedModelRecord>(&raw) {
                if record.active {
                    return Some(record.name);
                }
            }
        }
    }
    None
}

fn config_dir() -> PathBuf {
    env::var_os("OPENGPU_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|dir| dir.join(".opengpu")))
        .unwrap_or_else(|| PathBuf::from(".opengpu"))
}

fn trusted_runtime_paths_path() -> PathBuf {
    env::var_os("OPENGPU_TRUSTED_RUNTIME_PATHS")
        .map(PathBuf::from)
        .unwrap_or_else(|| config_dir().join("trusted-runtime-paths.json"))
}

fn load_trusted_runtime_paths() -> Result<Option<TrustedRuntimePaths>, String> {
    let path = trusted_runtime_paths_path();
    if !path.exists() {
        return Ok(None);
    }

    let raw = fs::read_to_string(&path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let config = serde_json::from_str::<TrustedRuntimePaths>(&raw)
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))?;
    Ok(Some(config))
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path)
        .map_err(|error| format!("failed to open {}: {error}", path.display()))?;
    let mut hasher = sha2::Sha256::new();
    let mut buffer = [0_u8; 8192];

    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
        if read == 0 {
            break;
        }
        sha2::Digest::update(&mut hasher, &buffer[..read]);
    }

    Ok(format!("{:x}", sha2::Digest::finalize(hasher)))
}

fn verify_trusted_executable(name: &str, pinned: &TrustedExecutable) -> Result<PathBuf, String> {
    let path = PathBuf::from(&pinned.path);
    if !path.is_absolute() {
        return Err(format!(
            "untrusted {name}: pinned runtime path must be absolute"
        ));
    }
    if !path.is_file() {
        return Err(format!(
            "missing {name}: trusted runtime path {} does not exist",
            path.display()
        ));
    }
    if let Some(expected) = pinned.sha256.as_deref() {
        let actual = sha256_file(&path)?;
        if !actual.eq_ignore_ascii_case(expected.trim()) {
            return Err(format!(
                "untrusted {name}: trusted runtime hash changed for {}",
                path.display()
            ));
        }
    }

    Ok(path)
}

fn trusted_runtime_executable(name: &str) -> Result<PathBuf, String> {
    let trusted = load_trusted_runtime_paths()?;
    let pinned = trusted.as_ref().and_then(|paths| match name {
        "llama-cli" => paths.llama_cli.as_ref(),
        "llama-server" => paths.llama_server.as_ref(),
        "nvidia-smi" => paths.nvidia_smi.as_ref(),
        _ => None,
    });

    match pinned {
        Some(pinned) => verify_trusted_executable(name, pinned),
        None if name == "llama-server" => {
            if let Some(cli) = trusted.as_ref().and_then(|paths| paths.llama_cli.as_ref()) {
                let cli_path = verify_trusted_executable("llama-cli", cli)?;
                if let Some(runtime_dir) = cli_path.parent() {
                    let sibling = runtime_dir.join(platform_executable_name("llama-server"));
                    if sibling.is_file() {
                        return Ok(sibling);
                    }
                }
            }
            Ok(PathBuf::from(name))
        }
        None => Ok(PathBuf::from(name)),
    }
}

fn platform_executable_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

fn imported_model_path_from_cache(model_dir: &Path, model_name: Option<&str>) -> Option<PathBuf> {
    let manifest_dir = model_dir.join(".opengpu");
    let entries = fs::read_dir(manifest_dir).ok()?;
    let mut active_fallback = None;

    for entry in entries.flatten() {
        if entry.file_type().ok()?.is_file()
            && entry.path().extension().and_then(|value| value.to_str()) == Some("json")
        {
            let raw = fs::read_to_string(entry.path()).ok()?;
            let record = serde_json::from_str::<CachedModelRecord>(&raw).ok()?;
            let path = record
                .source_path
                .as_ref()
                .map(PathBuf::from)
                .filter(|path| path.is_file())
                .or_else(|| cached_model_file_from_manifest(model_dir, &record));

            if model_name
                .map(|name| record.name == name)
                .unwrap_or(record.active)
            {
                if path.is_some() {
                    return path;
                }
            } else if record.active {
                active_fallback = path;
            }
        }
    }

    active_fallback
}

fn cached_model_file_from_manifest(
    model_dir: &Path,
    record: &CachedModelRecord,
) -> Option<PathBuf> {
    record
        .file_name
        .as_ref()
        .map(|file_name| {
            model_dir
                .join(sanitize_model_name(&record.name))
                .join(file_name)
        })
        .filter(|path| path.is_file())
}

fn model_has_reliable_structured_output(record: &CachedModelRecord) -> bool {
    if record.supports_structured_output
        || record
            .specialties
            .iter()
            .any(|specialty| matches!(specialty.as_str(), "speakai" | "structured_output"))
    {
        return true;
    }

    let name = record.name.to_ascii_lowercase();
    name.contains("qwen") || name.contains("phi-4") || name.contains("gemma")
}

fn preferred_speakai_model_name(
    model_dir: &Path,
    requested: Option<&str>,
    speakai: bool,
) -> Option<String> {
    if let Some(requested) = requested.map(str::trim).filter(|name| !name.is_empty()) {
        return Some(requested.to_string());
    }
    if !speakai {
        return active_model_name_from_cache(model_dir);
    }

    let manifest_dir = model_dir.join(".opengpu");
    let mut records = fs::read_dir(manifest_dir)
        .ok()?
        .flatten()
        .filter_map(|entry| {
            if !entry.file_type().ok()?.is_file()
                || entry.path().extension().and_then(|value| value.to_str()) != Some("json")
            {
                return None;
            }
            let raw = fs::read_to_string(entry.path()).ok()?;
            let record = serde_json::from_str::<CachedModelRecord>(&raw).ok()?;
            let usable = record
                .compatibility
                .as_deref()
                .is_none_or(|compatibility| compatibility != "rejected")
                && (record
                    .source_path
                    .as_deref()
                    .map(Path::new)
                    .is_some_and(Path::is_file)
                    || cached_model_file_from_manifest(model_dir, &record).is_some());
            usable.then_some(record)
        })
        .collect::<Vec<_>>();

    records.sort_by(|left, right| {
        let score = |record: &CachedModelRecord| {
            let speakai_specialty = record
                .specialties
                .iter()
                .any(|specialty| specialty == "speakai") as u8;
            (
                speakai_specialty,
                model_has_reliable_structured_output(record) as u8,
                record.supports_structured_output as u8,
                record.active as u8,
                record.estimated_vram_mb.unwrap_or(0),
            )
        };
        score(right)
            .cmp(&score(left))
            .then_with(|| left.name.cmp(&right.name))
    });

    records
        .into_iter()
        .find(model_has_reliable_structured_output)
        .or_else(|| {
            fs::read_dir(model_dir.join(".opengpu"))
                .ok()?
                .flatten()
                .filter_map(|entry| {
                    let raw = fs::read_to_string(entry.path()).ok()?;
                    serde_json::from_str::<CachedModelRecord>(&raw).ok()
                })
                .find(|record| record.active)
        })
        .map(|record| record.name)
}

pub fn active_model_capability(
    model_dir: &Path,
    model_name: Option<&str>,
) -> Option<ModelCapability> {
    let entries = fs::read_dir(model_dir.join(".opengpu")).ok()?;
    let mut fallback = None;
    for entry in entries.flatten() {
        let record = fs::read_to_string(entry.path())
            .ok()
            .and_then(|raw| serde_json::from_str::<CachedModelRecord>(&raw).ok())?;
        let capability = ModelCapability {
            name: record.name.clone(),
            path: record.source_path.clone().or_else(|| {
                cached_model_file_from_manifest(model_dir, &record)
                    .map(|path| path.display().to_string())
            }),
            format: record.format.clone(),
            quantization: record.quantization.clone(),
            size_bytes: record.size_bytes,
            estimated_vram_mb: record.estimated_vram_mb,
            compatibility: record.compatibility.clone(),
            compatibility_reason: record.compatibility_reason.clone(),
            active: record.active,
            languages: record.languages.clone(),
            specialties: record.specialties.clone(),
            supports_structured_output: record.supports_structured_output,
            ..ModelCapability::default()
        };
        if model_name
            .map(|name| record.name == name)
            .unwrap_or(record.active)
        {
            return Some(capability);
        }
        if record.active {
            fallback = Some(capability);
        }
    }
    fallback
}

pub fn available_model_capabilities(model_dir: &Path) -> Vec<ModelCapability> {
    let manifest_dir = model_dir.join(".opengpu");
    let Some(entries) = fs::read_dir(manifest_dir).ok() else {
        return Vec::new();
    };
    let mut capabilities = Vec::new();

    for entry in entries.flatten() {
        if entry.file_type().is_ok_and(|kind| kind.is_file())
            && entry.path().extension().and_then(|value| value.to_str()) == Some("json")
        {
            let Some(record) = fs::read_to_string(entry.path())
                .ok()
                .and_then(|raw| serde_json::from_str::<CachedModelRecord>(&raw).ok())
            else {
                continue;
            };
            let resolved_path = record.source_path.clone().or_else(|| {
                cached_model_file_from_manifest(model_dir, &record)
                    .map(|path| path.display().to_string())
            });
            if resolved_path.is_none() || record.compatibility.as_deref() == Some("rejected") {
                continue;
            }
            capabilities.push(ModelCapability {
                name: record.name.clone(),
                path: resolved_path,
                format: record.format.clone(),
                quantization: record.quantization.clone(),
                size_bytes: record.size_bytes,
                estimated_vram_mb: record.estimated_vram_mb,
                compatibility: record.compatibility.clone(),
                compatibility_reason: record.compatibility_reason.clone(),
                active: record.active,
                warm: false,
                languages: record.languages.clone(),
                specialties: record.specialties.clone(),
                supports_structured_output: record.supports_structured_output,
                ..ModelCapability::default()
            });
        }
    }
    capabilities.sort_by(|left, right| left.name.cmp(&right.name));
    capabilities
}

fn collect_gguf_files(dir: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
    if !dir.exists() {
        return Ok(());
    }

    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            collect_gguf_files(&path, files)?;
        } else if path.extension().and_then(|value| value.to_str()) == Some("gguf") {
            files.push(path);
        }
    }

    Ok(())
}

fn resolve_model_path(model_dir: &Path, model_name: Option<&str>) -> io::Result<PathBuf> {
    if let Some(path) = imported_model_path_from_cache(model_dir, model_name) {
        return Ok(path);
    }

    let mut search_dirs = Vec::new();
    if let Some(name) = model_name {
        search_dirs.push(model_dir.join(sanitize_model_name(name)));
    } else if let Some(active_name) = active_model_name_from_cache(model_dir) {
        search_dirs.push(model_dir.join(sanitize_model_name(&active_name)));
    }
    if search_dirs.is_empty() {
        search_dirs.push(model_dir.to_path_buf());
    }

    let mut files = Vec::new();
    for dir in search_dirs {
        collect_gguf_files(&dir, &mut files)?;
        if !files.is_empty() {
            break;
        }
    }

    files.sort();
    files
        .into_iter()
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no cached GGUF model found"))
}

fn probe_llama_cli_devices() -> Result<String, String> {
    let llama_cli = trusted_runtime_executable("llama-cli")?;
    let output = Command::new(&llama_cli)
        .arg("--list-devices")
        .output()
        .map_err(|error| format!("failed to launch llama-cli: {error}"))?;

    let stdout = String::from_utf8(output.stdout).map_err(|error| error.to_string())?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "llama-cli --list-devices exited {} — {}",
            output.status.code().unwrap_or(-1),
            stderr.lines().next().unwrap_or("no stderr")
        ));
    }

    Ok(stdout)
}

fn vulkan_device_name(devices: &str) -> Option<String> {
    devices
        .lines()
        .map(str::trim)
        .find(|line| line.to_ascii_lowercase().contains("vulkan"))
        .map(|line| {
            line.split_once(':')
                .map(|(_, name)| name.trim())
                .filter(|name| !name.is_empty())
                .unwrap_or(line)
                .to_string()
        })
}

pub fn probe_vulkan_device() -> Result<String, String> {
    let devices = probe_llama_cli_devices()?;
    vulkan_device_name(&devices).ok_or_else(|| {
        "Vulkan device not listed by llama-cli; install a Vulkan-capable graphics driver or use the CPU backend"
            .to_string()
    })
}

fn probe_llama_cli_available() -> Result<(), String> {
    let llama_cli = trusted_runtime_executable("llama-cli")?;
    let output = Command::new(&llama_cli)
        .arg("--version")
        .output()
        .map_err(|error| format!("failed to launch llama-cli: {error}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "llama-cli --version exited {} — {}",
            output.status.code().unwrap_or(-1),
            stderr.lines().next().unwrap_or("no stderr")
        ));
    }

    Ok(())
}

fn probe_llama_server_executable() -> Result<PathBuf, String> {
    trusted_runtime_executable("llama-server")
}

fn opengpu_home_dir() -> PathBuf {
    std::env::var_os("OPENGPU_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|dir| dir.join(".opengpu")))
        .unwrap_or_else(|| PathBuf::from(".opengpu"))
}

fn mlx_runtime_python_path() -> PathBuf {
    if let Some(path) = std::env::var_os("OPENGPU_MLX_PYTHON") {
        return PathBuf::from(path);
    }

    let mut path = opengpu_home_dir().join("runtimes").join("mlx").join("venv");
    #[cfg(windows)]
    {
        path = path.join("Scripts").join("python.exe");
    }
    #[cfg(not(windows))]
    {
        path = path.join("bin").join("python");
    }
    path
}

fn probe_mlx_available() -> Result<PathBuf, String> {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        let python = mlx_runtime_python_path();
        if !python.exists() {
            return Err(format!(
                "MLX runtime python not found at {}",
                python.display()
            ));
        }
        let output = Command::new(&python)
            .args(["-c", "import mlx_lm"])
            .output()
            .map_err(|error| format!("failed to launch MLX runtime: {error}"))?;
        if output.status.success() {
            return Ok(python);
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "MLX runtime import failed: {}",
            stderr
                .lines()
                .find(|line| !line.trim().is_empty())
                .unwrap_or("no stderr")
        ));
    }

    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    {
        Err("MLX is supported only on Apple Silicon macOS nodes".to_string())
    }
}

fn configured_llama_server_url() -> Option<String> {
    env::var("OPENGPU_LLAMA_SERVER_URL")
        .ok()
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| value.starts_with("http://") || value.starts_with("https://"))
}

fn configured_mlx_server_url() -> Option<String> {
    env::var("OPENGPU_MLX_SERVER_URL")
        .ok()
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| value.starts_with("http://") || value.starts_with("https://"))
}

fn vllm_runtime_config_path() -> PathBuf {
    opengpu_home_dir()
        .join("runtimes")
        .join("vllm")
        .join("runtime.conf")
}

fn vllm_runtime_setting(key: &str) -> Option<String> {
    let raw = fs::read_to_string(vllm_runtime_config_path()).ok()?;
    raw.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| line.split_once('='))
        .find(|(candidate, _)| candidate.trim() == key)
        .map(|(_, value)| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn vllm_setting(environment_key: &str, config_key: &str, default: &str) -> String {
    env::var(environment_key)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| vllm_runtime_setting(config_key))
        .unwrap_or_else(|| default.to_string())
}

fn configured_vllm_url() -> Option<String> {
    env::var("OPENGPU_VLLM_URL")
        .ok()
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| value.starts_with("http://") || value.starts_with("https://"))
        .or_else(|| {
            if !vllm_runtime_config_path().is_file() {
                return None;
            }
            let port = vllm_runtime_setting("VLLM_PORT").unwrap_or_else(|| "8000".to_string());
            Some(format!("http://127.0.0.1:{port}"))
        })
}

fn llama_server_health_ok(url: &str) -> bool {
    ureq::get(&format!("{url}/health"))
        .timeout(Duration::from_secs(2))
        .call()
        .map(|response| response.status() < 500)
        .unwrap_or(false)
}

fn mlx_server_health_ok(url: &str) -> bool {
    ureq::get(&format!("{url}/v1/models"))
        .timeout(Duration::from_secs(2))
        .call()
        .map(|response| response.status() < 400)
        .unwrap_or(false)
}

/// The running cluster this node contributes, when one was recorded by the CLI.
pub fn contributed_cluster() -> Option<crate::storage::ContributedCluster> {
    crate::storage::load_agent_config()
        .ok()
        .flatten()?
        .contributed_cluster
}

/// A contributed cluster is healthy when its endpoint still answers a model
/// listing. `/health` is accepted as a fallback for runtimes that do not expose
/// `/v1/models` without auth.
pub fn cluster_endpoint_healthy(base_url: &str) -> bool {
    let base_url = base_url.trim_end_matches('/');
    for path in ["/v1/models", "/api/tags", "/health"] {
        let ok = ureq::get(&format!("{base_url}{path}"))
            .timeout(Duration::from_secs(2))
            .call()
            .map(|response| response.status() < 400)
            .unwrap_or(false);
        if ok {
            return true;
        }
    }
    false
}

fn vllm_health_ok(url: &str) -> bool {
    ureq::get(&format!("{url}/health"))
        .timeout(Duration::from_secs(2))
        .call()
        .map(|response| response.status() < 500)
        .unwrap_or(false)
}

pub struct PersistentRuntimeHandle {
    child: Child,
    url: String,
    environment_variable: &'static str,
    container_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct PersistentRuntimeState {
    pid: u32,
    url: String,
    environment_variable: String,
    #[serde(default)]
    container_name: Option<String>,
    updated_at: String,
}

impl PersistentRuntimeHandle {
    fn new(
        child: Child,
        url: String,
        environment_variable: &'static str,
        container_name: Option<String>,
    ) -> Self {
        let handle = Self {
            child,
            url,
            environment_variable,
            container_name,
        };
        let _ = write_persistent_runtime_state(&handle);
        handle
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn environment_variable(&self) -> &'static str {
        self.environment_variable
    }
}

impl Drop for PersistentRuntimeHandle {
    fn drop(&mut self) {
        stop_persistent_runtime_process(self.child.id(), self.container_name.as_deref());
        let _ = self.child.wait();
        remove_persistent_runtime_state();
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistentRuntimeStop {
    pub pid: u32,
    pub url: String,
    pub container_name: Option<String>,
}

fn persistent_runtime_state_path() -> PathBuf {
    config_dir().join("persistent-runtime.json")
}

fn write_persistent_runtime_state(handle: &PersistentRuntimeHandle) -> Result<(), String> {
    let path = persistent_runtime_state_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create runtime state directory: {error}"))?;
    }
    let state = PersistentRuntimeState {
        pid: handle.child.id(),
        url: handle.url.clone(),
        environment_variable: handle.environment_variable.to_string(),
        container_name: handle.container_name.clone(),
        updated_at: now_unix_seconds(),
    };
    let payload = serde_json::to_string_pretty(&state)
        .map_err(|error| format!("failed to serialize runtime state: {error}"))?;
    fs::write(&path, format!("{payload}\n")).map_err(|error| {
        format!(
            "failed to write runtime state `{}`: {error}",
            path.display()
        )
    })
}

fn remove_persistent_runtime_state() {
    let _ = fs::remove_file(persistent_runtime_state_path());
}

fn read_persistent_runtime_state() -> Result<Option<PersistentRuntimeState>, String> {
    let path = persistent_runtime_state_path();
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&path)
        .map_err(|error| format!("failed to read runtime state `{}`: {error}", path.display()))?;
    let state = serde_json::from_str::<PersistentRuntimeState>(&raw).map_err(|error| {
        format!(
            "failed to parse runtime state `{}`: {error}",
            path.display()
        )
    })?;
    Ok(Some(state))
}

fn stop_persistent_runtime_process(pid: u32, container_name: Option<&str>) {
    if let Some(container_name) = container_name {
        let docker = env::var_os("OPENGPU_DOCKER_BIN").unwrap_or_else(|| "docker".into());
        let _ = Command::new(docker)
            .args(["stop", "--timeout", "10", container_name])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    if !runtime_pid_is_safe_to_kill(pid) {
        return;
    }
    kill_process_tree(pid);
}

#[cfg(windows)]
fn runtime_pid_is_safe_to_kill(pid: u32) -> bool {
    pid > 0
}

#[cfg(not(windows))]
fn runtime_pid_is_safe_to_kill(pid: u32) -> bool {
    pid > 0 && pid <= i32::MAX as u32
}

pub fn stop_persistent_runtime_from_state() -> Result<Option<PersistentRuntimeStop>, String> {
    let Some(state) = read_persistent_runtime_state()? else {
        return Ok(None);
    };
    stop_persistent_runtime_process(state.pid, state.container_name.as_deref());
    remove_persistent_runtime_state();
    Ok(Some(PersistentRuntimeStop {
        pid: state.pid,
        url: state.url,
        container_name: state.container_name,
    }))
}

fn percentage_token(line: &str) -> Option<&str> {
    line.split_whitespace().rev().find_map(|token| {
        let percent_index = token.find('%')?;
        let start = token[..percent_index]
            .rfind(|ch: char| !ch.is_ascii_digit())
            .map(|index| index + 1)
            .unwrap_or(0);
        (start < percent_index).then(|| &token[start..=percent_index])
    })
}

fn shard_fraction(line: &str) -> Option<&str> {
    line.split_whitespace().find(|token| {
        let Some((loaded, total)) = token.split_once('/') else {
            return false;
        };
        loaded.chars().all(|ch| ch.is_ascii_digit()) && total.chars().all(|ch| ch.is_ascii_digit())
    })
}

fn vllm_startup_progress(line: &str) -> Option<String> {
    if line.contains("Loading safetensors checkpoint shards:") {
        let percent = percentage_token(line)?;
        let shards = shard_fraction(line)?;
        return Some(format!("loading checkpoint shards {shards} ({percent})"));
    }
    if line.contains("Downloading (incomplete total") {
        return percentage_token(line)
            .map(|percent| format!("downloading model files ({percent})"));
    }
    if line.contains("Fetching ") && line.contains(" files:") {
        return percentage_token(line).map(|percent| format!("fetching model files ({percent})"));
    }
    if line.contains("Parse safetensors files:") {
        return percentage_token(line).map(|percent| format!("indexing model files ({percent})"));
    }
    if line.contains("Model loading took") {
        return Some("model weights loaded; preparing KV cache".to_string());
    }
    if line.contains("init engine (profile, create kv cache, warmup model) took") {
        return Some("CUDA and KV-cache warm-up complete".to_string());
    }
    if line.contains("Application startup complete") {
        return Some("ready (100%)".to_string());
    }
    None
}

fn emit_vllm_startup_progress(
    log_path: &Path,
    log_offset: &mut u64,
    last_progress: &mut Option<String>,
) {
    let Ok(mut file) = OpenOptions::new().read(true).open(log_path) else {
        return;
    };
    if file.seek(SeekFrom::Start(*log_offset)).is_err() {
        return;
    }
    let mut bytes = Vec::new();
    if file.read_to_end(&mut bytes).is_err() {
        return;
    }
    *log_offset = log_offset.saturating_add(bytes.len() as u64);
    let text = String::from_utf8_lossy(&bytes);
    for line in text.split(['\r', '\n']) {
        let Some(progress) = vllm_startup_progress(line) else {
            continue;
        };
        if last_progress.as_ref() == Some(&progress) {
            continue;
        }
        println!("vllmStartup: {progress}");
        *last_progress = Some(progress);
    }
}

fn start_vllm_runtime(
    model_dir: &Path,
    model_name: Option<&str>,
) -> Result<Option<PersistentRuntimeHandle>, String> {
    if !cfg!(target_os = "linux") {
        return Err("vLLM persistent runtime is supported only on Linux".to_string());
    }
    let model_name = model_name
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "vLLM persistent runtime requires an active model".to_string())?;
    let port = vllm_setting("OPENGPU_VLLM_PORT", "VLLM_PORT", "8000")
        .parse::<u16>()
        .ok()
        .filter(|port| *port > 0)
        .unwrap_or(8000);
    let url = format!("http://127.0.0.1:{port}");
    if vllm_health_ok(&url) {
        return Ok(None);
    }

    let image = vllm_setting(
        "OPENGPU_VLLM_IMAGE",
        "VLLM_IMAGE",
        "nvcr.io/nvidia/vllm@sha256:63b808804826a028e38f559747a9e4d5985cf676616fbaa70c1937c58f83e13e",
    );
    let container_name = vllm_setting(
        "OPENGPU_VLLM_CONTAINER_NAME",
        "VLLM_CONTAINER_NAME",
        "mundusx-vllm",
    );
    let memory_utilization = vllm_setting(
        "OPENGPU_VLLM_GPU_MEMORY_UTILIZATION",
        "VLLM_GPU_MEMORY_UTILIZATION",
        "0.70",
    );
    let max_num_seqs = vllm_setting("OPENGPU_VLLM_MAX_NUM_SEQS", "VLLM_MAX_NUM_SEQS", "4");
    let docker = env::var_os("OPENGPU_DOCKER_BIN").unwrap_or_else(|| "docker".into());
    let runtime_dir = opengpu_home_dir().join("runtimes").join("vllm");
    fs::create_dir_all(&runtime_dir)
        .map_err(|error| format!("failed to create vLLM runtime directory: {error}"))?;
    fs::create_dir_all(model_dir)
        .map_err(|error| format!("failed to create vLLM model directory: {error}"))?;
    let log_path = runtime_dir.join("runtime.log");
    let mut log_offset = fs::metadata(&log_path)
        .map(|metadata| metadata.len())
        .unwrap_or_default();
    let mut last_progress = None;
    let stdout = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|error| format!("failed to open vLLM runtime log: {error}"))?;
    let stderr = stdout
        .try_clone()
        .map_err(|error| format!("failed to clone vLLM runtime log: {error}"))?;

    let mut command = Command::new(docker);
    command
        .args([
            "run",
            "--rm",
            "--gpus",
            "all",
            "--ipc=host",
            "--ulimit",
            "memlock=-1",
            "--ulimit",
            "stack=67108864",
            "--name",
        ])
        .arg(&container_name)
        .arg("-p")
        .arg(format!("127.0.0.1:{port}:8000"))
        .arg("-v")
        .arg(format!("{}:/models", model_dir.display()))
        .args(["-e", "HF_HOME=/models/.huggingface"]);
    if env::var_os("HF_TOKEN").is_some() {
        command.args(["-e", "HF_TOKEN"]);
    }
    command
        .arg(&image)
        .args(["vllm", "serve"])
        .arg(model_name)
        .args(["--host", "0.0.0.0", "--port", "8000"])
        .arg("--gpu-memory-utilization")
        .arg(&memory_utilization)
        .arg("--max-num-seqs")
        .arg(&max_num_seqs)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));

    let mut child = command
        .spawn()
        .map_err(|error| format!("failed to launch vLLM container: {error}"))?;
    let timeout_seconds = vllm_setting(
        "OPENGPU_VLLM_START_TIMEOUT_SECONDS",
        "VLLM_START_TIMEOUT_SECONDS",
        "1800",
    )
    .parse::<u64>()
    .ok()
    .filter(|seconds| *seconds > 0)
    .unwrap_or(1800);
    let deadline = Instant::now() + Duration::from_secs(timeout_seconds);
    while Instant::now() < deadline {
        emit_vllm_startup_progress(&log_path, &mut log_offset, &mut last_progress);
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("failed to poll vLLM container: {error}"))?
        {
            return Err(format!(
                "vLLM container exited during startup with {}; see {}",
                status.code().unwrap_or(-1),
                log_path.display()
            ));
        }
        if vllm_health_ok(&url) {
            if last_progress.as_deref() != Some("ready (100%)") {
                println!("vllmStartup: ready (100%)");
            }
            return Ok(Some(PersistentRuntimeHandle::new(
                child,
                url,
                "OPENGPU_VLLM_URL",
                Some(container_name),
            )));
        }
        thread::sleep(Duration::from_secs(1));
    }

    let _ = Command::new(env::var_os("OPENGPU_DOCKER_BIN").unwrap_or_else(|| "docker".into()))
        .args(["stop", "--timeout", "10", &container_name])
        .status();
    kill_process_tree(child.id());
    let _ = child.wait();
    Err(format!(
        "vLLM runtime did not become healthy within {timeout_seconds}s; see {}",
        log_path.display()
    ))
}

pub fn start_persistent_runtime(
    model_dir: &Path,
    model_name: Option<&str>,
    backend: Backend,
    parallel_slots: u8,
) -> Result<Option<PersistentRuntimeHandle>, String> {
    if env::var("OPENGPU_PERSISTENT_RUNTIME")
        .map(|value| value.eq_ignore_ascii_case("off") || value.eq_ignore_ascii_case("false"))
        .unwrap_or(false)
    {
        return Ok(None);
    }
    if backend == Backend::Vllm {
        return start_vllm_runtime(model_dir, model_name);
    }
    if backend == Backend::M {
        let Some(model_name) = model_name.map(str::trim).filter(|name| !name.is_empty()) else {
            return Ok(None);
        };
        if let Ok(python) = probe_mlx_available() {
            let port = env::var("OPENGPU_MLX_SERVER_PORT")
                .ok()
                .and_then(|value| value.parse::<u16>().ok())
                .filter(|port| *port > 0)
                .unwrap_or(8790);
            let url = format!("http://127.0.0.1:{port}");
            if mlx_server_health_ok(&url) {
                env::set_var("OPENGPU_MLX_SERVER_URL", &url);
                return Ok(None);
            }

            let log_path = opengpu_home_dir()
                .join("runtimes")
                .join("mlx")
                .join("server.log");
            if let Some(parent) = log_path.parent() {
                fs::create_dir_all(parent)
                    .map_err(|error| format!("failed to create MLX runtime directory: {error}"))?;
            }
            let log = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_path)
                .map_err(|error| format!("failed to open MLX runtime log: {error}"))?;
            let stdout = log
                .try_clone()
                .map_err(|error| format!("failed to clone MLX runtime log handle: {error}"))?;
            let mut child = Command::new(python)
                .args([
                    "-m",
                    "mlx_lm.server",
                    "--model",
                    model_name,
                    "--host",
                    "127.0.0.1",
                    "--port",
                    &port.to_string(),
                ])
                .stdin(Stdio::null())
                .stdout(Stdio::from(stdout))
                .stderr(Stdio::from(log))
                .spawn()
                .map_err(|error| format!("failed to launch persistent MLX runtime: {error}"))?;
            let timeout_seconds = env::var("OPENGPU_MLX_START_TIMEOUT_SECONDS")
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                .filter(|seconds| *seconds > 0)
                .unwrap_or(300);
            let deadline = Instant::now() + Duration::from_secs(timeout_seconds);
            while Instant::now() < deadline {
                if let Some(status) = child
                    .try_wait()
                    .map_err(|error| format!("failed to poll persistent MLX runtime: {error}"))?
                {
                    return Err(format!(
                        "persistent MLX runtime exited during startup with {}; see {}",
                        status.code().unwrap_or(-1),
                        log_path.display()
                    ));
                }
                if mlx_server_health_ok(&url) {
                    return Ok(Some(PersistentRuntimeHandle::new(
                        child,
                        url,
                        "OPENGPU_MLX_SERVER_URL",
                        None,
                    )));
                }
                thread::sleep(Duration::from_millis(500));
            }

            kill_process_tree(child.id());
            let _ = child.wait();
            return Err(format!(
                "persistent MLX runtime did not become healthy within {timeout_seconds}s; see {}",
                log_path.display()
            ));
        }
    }

    let Some(model_path) = resolve_model_path(model_dir, model_name).ok() else {
        return Ok(None);
    };
    let llama_server = match probe_llama_server_executable() {
        Ok(path) => path,
        Err(_) => return Ok(None),
    };
    let port = env::var("OPENGPU_LLAMA_SERVER_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .filter(|port| *port > 0)
        .unwrap_or(8789);
    let url = format!("http://127.0.0.1:{port}");
    if llama_server_health_ok(&url) {
        return Ok(None);
    }

    let mut command = Command::new(llama_server);
    command
        .arg("-m")
        .arg(model_path)
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(port.to_string())
        .arg("-c")
        .arg((4096_u32 * u32::from(parallel_slots.max(1))).to_string())
        .arg("--parallel")
        .arg(parallel_slots.max(1).to_string())
        .arg("--threads")
        .arg("2")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if matches!(backend, Backend::Cuda) {
        command.arg("--device").arg("CUDA0");
    } else if matches!(backend, Backend::Vulkan) {
        command
            .arg("--device")
            .arg("Vulkan0")
            .arg("--gpu-layers")
            .arg("99");
    }

    let mut child = command
        .spawn()
        .map_err(|error| format!("failed to launch llama-server: {error}"))?;
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("failed to poll llama-server: {error}"))?
        {
            return Err(format!(
                "llama-server exited during startup with {}",
                status.code().unwrap_or(-1)
            ));
        }
        if llama_server_health_ok(&url) {
            return Ok(Some(PersistentRuntimeHandle::new(
                child,
                url,
                "OPENGPU_LLAMA_SERVER_URL",
                None,
            )));
        }
        thread::sleep(Duration::from_millis(300));
    }

    kill_process_tree(child.id());
    let _ = child.wait();
    Err("llama-server did not become healthy within 20s".to_string())
}

pub fn recommended_parallel_slots(
    backend: Backend,
    physical_vram_mb: Option<u32>,
    unified_memory_mb: Option<u32>,
    contribution_percent: u8,
    model_name: Option<&str>,
) -> u8 {
    let capacity_memory_mb = match backend {
        Backend::Cuda => physical_vram_mb,
        Backend::M | Backend::Vllm => unified_memory_mb,
        _ => None,
    };
    let Some(capacity_memory_mb) = capacity_memory_mb else {
        return 1;
    };
    let usable_memory_mb = capacity_memory_mb
        .saturating_mul(u32::from(contribution_percent))
        .saturating_add(99)
        / 100;
    let mut slots = match backend {
        Backend::Cuda => match usable_memory_mb {
            0..=8_192 => 1,
            8_193..=16_384 => 2,
            16_385..=24_575 => 3,
            _ => 4,
        },
        Backend::M | Backend::Vllm => match usable_memory_mb {
            0..=16_384 => 1,
            16_385..=32_768 => 2,
            32_769..=65_536 => 3,
            _ => 4,
        },
        _ => 1,
    };

    let model = model_name.unwrap_or_default().to_ascii_lowercase();
    if ["32b", "34b", "65b", "70b", "72b"]
        .iter()
        .any(|marker| model.contains(marker))
    {
        slots = 1;
    } else if ["13b", "14b"].iter().any(|marker| model.contains(marker)) {
        slots = slots.min(2);
    } else if ["7b", "8b"].iter().any(|marker| model.contains(marker)) {
        slots = slots.min(3);
    }
    slots.max(1)
}

fn parse_nvidia_smi_query(stdout: &str) -> CudaDiagnostics {
    let mut best: Option<(String, u32)> = None;
    let mut detected_device_name = None;

    for line in stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let Some((name, memory)) = line.rsplit_once(',') else {
            continue;
        };
        let name = name.trim().to_string();
        if !name.is_empty() && detected_device_name.is_none() {
            detected_device_name = Some(name.clone());
        }
        let Some(memory_mb) = memory.trim().parse::<u32>().ok() else {
            continue;
        };
        if best
            .as_ref()
            .map(|(_, best_memory)| memory_mb > *best_memory)
            .unwrap_or(true)
        {
            best = Some((name, memory_mb));
        }
    }

    if let Some((device_name, memory_mb)) = best {
        let low_vram_profile = memory_mb <= 4096;
        let mut notes = Vec::new();
        if low_vram_profile {
            notes.push(format!(
                "CUDA low-VRAM profile selected for {memory_mb} MB; advertise modest workloads only"
            ));
        }

        CudaDiagnostics {
            device_available: true,
            driver_available: true,
            device_name: Some(device_name),
            memory_mb: Some(memory_mb),
            low_vram_profile,
            notes,
        }
    } else if let Some(device_name) = detected_device_name {
        CudaDiagnostics {
            device_available: true,
            driver_available: true,
            device_name: Some(device_name),
            memory_mb: None,
            low_vram_profile: false,
            notes: vec![
                "nvidia-smi reported a GPU without dedicated VRAM; using unified system memory"
                    .to_string(),
            ],
        }
    } else {
        CudaDiagnostics {
            driver_available: true,
            notes: vec!["nvidia-smi returned no parseable GPU rows".to_string()],
            ..CudaDiagnostics::default()
        }
    }
}

pub fn probe_cuda_diagnostics() -> CudaDiagnostics {
    let nvidia_smi = match trusted_runtime_executable("nvidia-smi") {
        Ok(path) => path,
        Err(error) => {
            return CudaDiagnostics {
                notes: vec![error],
                ..CudaDiagnostics::default()
            };
        }
    };
    let output = Command::new(&nvidia_smi)
        .args([
            "--query-gpu=name,memory.total",
            "--format=csv,noheader,nounits",
        ])
        .output();

    match output {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            parse_nvidia_smi_query(&stdout)
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            CudaDiagnostics {
                notes: vec![format!(
                    "nvidia-smi exited {}: {}",
                    output.status.code().unwrap_or(-1),
                    stderr.lines().next().unwrap_or("no stderr")
                )],
                ..CudaDiagnostics::default()
            }
        }
        Err(error) => CudaDiagnostics {
            notes: vec![format!(
                "nvidia-smi unavailable; install NVIDIA driver/CUDA runtime first: {error}"
            )],
            ..CudaDiagnostics::default()
        },
    }
}

fn probe_power_state() -> PowerState {
    #[cfg(target_os = "macos")]
    {
        let output = Command::new("pmset").args(["-g", "batt"]).output();
        if let Ok(output) = output {
            if let Ok(stdout) = String::from_utf8(output.stdout) {
                let mut source = "unknown".to_string();
                let mut on_battery = true;
                let mut battery_percent = None;

                for line in stdout.lines() {
                    if line.starts_with("Now drawing from") {
                        source = line
                            .split_once('\'')
                            .map(|(_, rest)| rest.trim_matches('\'').to_string())
                            .unwrap_or_else(|| line.to_string());
                        on_battery = !source.to_lowercase().contains("ac power");
                    }
                    if let Some(percent_text) = line.split('%').next() {
                        if let Some(token) = percent_text
                            .split_whitespace()
                            .rev()
                            .find(|part| part.chars().all(|ch| ch.is_ascii_digit()))
                        {
                            battery_percent = token.parse::<u8>().ok();
                        }
                    }
                }

                return PowerState {
                    source,
                    on_battery,
                    battery_percent,
                };
            }
        }
    }

    PowerState {
        source: "unknown".to_string(),
        on_battery: false,
        battery_percent: None,
    }
}

fn run_llama_command(
    model_path: &Path,
    prompt: &str,
    backend: Backend,
    max_tokens: u32,
    temperature: f32,
    top_p: f32,
    seed: u64,
    grammar: Option<&str>,
) -> Result<(String, String, RuntimeMetrics), String> {
    if backend == Backend::Vllm {
        return Err(
            "vLLM execution is not enabled in this worker; use Linux vLLM nodes only after the vLLM runtime adapter is installed"
                .to_string(),
        );
    }

    let llama_cli = trusted_runtime_executable("llama-cli")?;
    let runtime_mode = match backend {
        Backend::Cuda => "cuda",
        Backend::Vulkan => "vulkan",
        _ => "blas",
    };
    let mut command = Command::new(&llama_cli);
    command.arg("-m").arg(model_path);
    if matches!(backend, Backend::Cuda) {
        command.arg("--device").arg("CUDA0");
    } else if matches!(backend, Backend::Vulkan) {
        command
            .arg("--device")
            .arg("Vulkan0")
            .arg("--gpu-layers")
            .arg("99");
    }
    command
        .arg("--no-conversation")
        .arg("--simple-io")
        .arg("--no-display-prompt")
        .arg("-c")
        .arg(context_size_for(prompt, max_tokens).to_string())
        .arg("--threads")
        .arg("2")
        .arg("--threads-batch")
        .arg("2")
        .arg("-p")
        .arg(prompt)
        .arg("-n")
        .arg(max_tokens.to_string())
        .arg("--temp")
        .arg(temperature.to_string())
        .arg("--top-p")
        .arg(top_p.to_string())
        .arg("--seed")
        .arg(seed.to_string());
    if let Some(grammar) = grammar {
        command.arg("--grammar").arg(grammar);
    }

    let output = command
        .output()
        .map_err(|error| format!("failed to launch llama-cli: {error}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "llama-cli exited {} — {}",
            output.status.code().unwrap_or(-1),
            first_actionable_stderr_line(&stderr)
        ));
    }

    let transcript = String::from_utf8(output.stdout)
        .map_err(|error| error.to_string())?
        .trim()
        .to_string();
    let stderr = String::from_utf8_lossy(&output.stderr);
    let diagnostics = format!("{transcript}\n{stderr}");
    let metrics = llama_perf_metrics(&diagnostics);
    let generated = extract_llama_response(prompt, &transcript);

    Ok((generated, runtime_mode.to_string(), metrics))
}

fn run_llama_server_completion(
    url: &str,
    system_prompt: &str,
    user_prompt: &str,
    backend: Backend,
    max_tokens: u32,
    temperature: f32,
    top_p: f32,
    seed: u64,
    structured: bool,
) -> Result<(String, String, RuntimeMetrics), String> {
    let mut messages = Vec::new();
    if !system_prompt.is_empty() {
        messages.push(serde_json::json!({
            "role": "system",
            "content": system_prompt,
        }));
    }
    messages.push(serde_json::json!({
        "role": "user",
        "content": user_prompt,
    }));
    let mut payload = serde_json::json!({
        "messages": messages,
        "max_tokens": max_tokens,
        "temperature": temperature,
        "top_p": top_p,
        "seed": seed,
    });
    if structured {
        payload["response_format"] = speakai_response_format();
    }
    let response = ureq::post(&format!("{url}/v1/chat/completions"))
        .timeout(Duration::from_secs(worker_timeout().as_secs().max(30)))
        .send_json(payload)
        .map_err(|error| format!("llama-server completion failed: {error}"))?;
    let value = response
        .into_json::<serde_json::Value>()
        .map_err(|error| format!("llama-server returned invalid json: {error}"))?;
    let generated = value
        .pointer("/choices/0/message/content")
        .or_else(|| value.get("content"))
        .or_else(|| value.get("response"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "llama-server response did not include content".to_string())?
        .to_string();
    let metrics = llama_server_metrics(&value);
    let runtime_mode = match backend {
        Backend::Cuda => "persistent-warm-cuda",
        Backend::Vulkan => "persistent-warm-vulkan",
        Backend::M => "persistent-warm-blas",
        _ => "persistent-warm",
    };
    Ok((generated, runtime_mode.to_string(), metrics))
}

fn run_vllm_completion(
    url: &str,
    model: &str,
    system_prompt: &str,
    user_prompt: &str,
    max_tokens: u32,
    temperature: f32,
    top_p: f32,
    seed: u64,
    structured: bool,
) -> Result<String, String> {
    let mut messages = Vec::new();
    if !system_prompt.is_empty() {
        messages.push(serde_json::json!({
            "role": "system",
            "content": system_prompt,
        }));
    }
    messages.push(serde_json::json!({
        "role": "user",
        "content": user_prompt,
    }));
    let mut payload = serde_json::json!({
        "model": model,
        "messages": messages,
        "max_tokens": max_tokens,
        "temperature": temperature,
        "top_p": top_p,
        "seed": seed,
    });
    let live_stream = live_delta_enabled() && !structured;
    if live_stream {
        payload["stream"] = serde_json::Value::Bool(true);
        payload["stream_options"] = serde_json::json!({"include_usage": true});
    }
    if structured {
        payload["response_format"] = speakai_response_format();
    }
    let response = ureq::post(&format!("{url}/v1/chat/completions"))
        .timeout(Duration::from_secs(worker_timeout().as_secs().max(30)))
        .send_json(payload)
        .map_err(|error| format!("OpenAI-compatible completion failed: {error}"))?;
    if live_stream {
        return parse_openai_stream(BufReader::new(response.into_reader()));
    }
    let value = response
        .into_json::<serde_json::Value>()
        .map_err(|error| format!("OpenAI-compatible runtime returned invalid json: {error}"))?;
    let finish_reason = value
        .pointer("/choices/0/finish_reason")
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .to_string();

    if let Some(content) = value
        .pointer("/choices/0/message/content")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        // A generation cut off by the token or context limit must never be
        // presented as a finished answer, so say so alongside the text.
        if finish_reason == "length" {
            return Ok(format!("[truncated: hit the generation limit] {content}"));
        }
        return Ok(content.to_string());
    }

    Err(empty_completion_error(&value))
}

fn char_prefix_bytes(value: &str, chars: usize) -> usize {
    value
        .char_indices()
        .nth(chars)
        .map(|(index, _)| index)
        .unwrap_or(value.len())
}

fn parse_openai_stream<R: BufRead>(reader: R) -> Result<String, String> {
    let mut raw_content = String::new();
    let mut emitted_chars = 0usize;
    let mut finish_reason = String::new();
    let mut saw_done = false;
    let mut last_emit = Instant::now();

    for line in reader.lines() {
        let line =
            line.map_err(|error| format!("OpenAI-compatible stream read failed: {error}"))?;
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let data = data.trim_start();
        if data == "[DONE]" {
            saw_done = true;
            break;
        }
        if data.is_empty() {
            continue;
        }
        let value: serde_json::Value = serde_json::from_str(data)
            .map_err(|error| format!("OpenAI-compatible stream returned invalid json: {error}"))?;
        if let Some(message) = value
            .pointer("/error/message")
            .and_then(serde_json::Value::as_str)
        {
            return Err(format!("runtime returned an error: {message}"));
        }
        if let Some(reason) = value
            .pointer("/choices/0/finish_reason")
            .and_then(serde_json::Value::as_str)
        {
            finish_reason = reason.to_string();
        }
        if let Some(delta) = value
            .pointer("/choices/0/delta/content")
            .and_then(serde_json::Value::as_str)
        {
            raw_content.push_str(delta);
            let normalized = raw_content.trim_start();
            let safe_chars = normalized
                .chars()
                .count()
                .saturating_sub(STREAM_TAIL_HOLD_CHARS);
            let pending_chars = safe_chars.saturating_sub(emitted_chars);
            if pending_chars > 0
                && (last_emit.elapsed() >= Duration::from_millis(30) || pending_chars >= 128)
            {
                let start = char_prefix_bytes(normalized, emitted_chars);
                let end = char_prefix_bytes(normalized, safe_chars);
                emit_stream_delta(&normalized[start..end]);
                emitted_chars = safe_chars;
                last_emit = Instant::now();
            }
        }
    }

    let content = raw_content.trim();
    if content.is_empty() {
        return Err("OpenAI-compatible stream did not include content".to_string());
    }
    if !saw_done && finish_reason.is_empty() {
        return Err("OpenAI-compatible stream ended before a terminal event".to_string());
    }
    let safe_chars = content
        .chars()
        .count()
        .saturating_sub(STREAM_TAIL_HOLD_CHARS);
    if safe_chars > emitted_chars {
        let start = char_prefix_bytes(content, emitted_chars);
        let end = char_prefix_bytes(content, safe_chars);
        emit_stream_delta(&content[start..end]);
    }
    if finish_reason == "length" {
        Ok(format!("[truncated: hit the generation limit] {content}"))
    } else {
        Ok(content.to_string())
    }
}

fn structured_output_option_unsupported(error: &str) -> bool {
    ["status code 400", "status code 404", "status code 422"]
        .iter()
        .any(|status| error.contains(status))
}

fn run_openai_compatible_completion(
    url: &str,
    model: &str,
    system_prompt: &str,
    user_prompt: &str,
    max_tokens: u32,
    temperature: f32,
    top_p: f32,
    seed: u64,
    structured: bool,
) -> Result<String, String> {
    match run_vllm_completion(
        url,
        model,
        system_prompt,
        user_prompt,
        max_tokens,
        temperature,
        top_p,
        seed,
        structured,
    ) {
        Err(error) if structured && structured_output_option_unsupported(&error) => {
            run_vllm_completion(
                url,
                model,
                system_prompt,
                user_prompt,
                max_tokens,
                temperature,
                top_p,
                seed,
                false,
            )
        }
        result => result,
    }
}

/// Explains an empty `content` instead of reporting a missing field.
///
/// Reasoning models emit `reasoning_content` first, so a small `max_tokens`
/// leaves `content` empty with `finish_reason: "length"`. That is a budget
/// problem, not a malformed response, and the message needs to say so.
fn empty_completion_error(value: &serde_json::Value) -> String {
    let finish_reason = value
        .pointer("/choices/0/finish_reason")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    let reasoning_only = value
        .pointer("/choices/0/message/reasoning_content")
        .and_then(|value| value.as_str())
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);

    let generated = value
        .pointer("/usage/completion_tokens")
        .and_then(serde_json::Value::as_u64);
    if finish_reason == "length" && reasoning_only {
        return format!(
            "model spent its whole budget on reasoning and returned no content{}; raise max_tokens, and check the runtime's context size if raising it does not help",
            generated
                .map(|count| format!(" ({count} tokens generated)"))
                .unwrap_or_default()
        );
    }
    if finish_reason == "length" {
        return "model hit the generation limit before producing content; raise max_tokens or the runtime's context size".to_string();
    }
    if reasoning_only {
        return "model returned only reasoning_content and no content".to_string();
    }
    if let Some(message) = value
        .pointer("/error/message")
        .and_then(|value| value.as_str())
    {
        return format!("runtime returned an error: {message}");
    }
    "response did not include choices[0].message.content".to_string()
}

fn run_mlx_command(
    model_name: &str,
    prompt: &str,
    max_tokens: u32,
    temperature: f32,
) -> Result<(String, String, RuntimeMetrics), String> {
    let python = probe_mlx_available()?;
    let started = Instant::now();
    let output = Command::new(&python)
        .args([
            "-m",
            "mlx_lm",
            "generate",
            "--model",
            model_name,
            "--prompt",
            prompt,
            "--max-tokens",
            &max_tokens.to_string(),
            "--temp",
            &temperature.to_string(),
        ])
        .output()
        .map_err(|error| format!("failed to launch MLX runtime: {error}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "mlx-lm exited {} — {}",
            output.status.code().unwrap_or(-1),
            first_actionable_stderr_line(&stderr)
        ));
    }

    let transcript = String::from_utf8(output.stdout)
        .map_err(|error| error.to_string())?
        .trim()
        .to_string();
    let generated = extract_mlx_response(&transcript);
    let metrics = RuntimeMetrics {
        total_duration_ms: Some(started.elapsed().as_secs_f64() * 1000.0),
        load_duration_ms: None,
        prompt_eval_count: None,
        prompt_eval_duration_ms: None,
        prompt_eval_rate: None,
        eval_count: None,
        eval_duration_ms: None,
        eval_rate: None,
    };

    Ok((generated, "mlx".to_string(), metrics))
}

fn extract_mlx_response(transcript: &str) -> String {
    let mut lines = Vec::new();
    for line in transcript.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("Fetching ")
            || trimmed.starts_with("Loading ")
            || trimmed.starts_with("========")
            || trimmed.starts_with("Prompt:")
            || trimmed.starts_with("Generation")
            || trimmed.starts_with("Peak memory")
        {
            continue;
        }
        if !trimmed.is_empty() {
            lines.push(trimmed);
        }
    }
    lines.join("\n").trim().to_string()
}

fn llama_server_metrics(value: &serde_json::Value) -> RuntimeMetrics {
    let timings = value.get("timings").unwrap_or(value);
    RuntimeMetrics {
        total_duration_ms: timing_number(timings, &["total_ms", "total_duration_ms"]),
        load_duration_ms: timing_number(timings, &["load_ms", "load_duration_ms"]),
        prompt_eval_count: timing_number(timings, &["prompt_n", "prompt_eval_count"])
            .map(|value| value.round() as u64),
        prompt_eval_duration_ms: timing_number(timings, &["prompt_ms", "prompt_eval_duration_ms"]),
        prompt_eval_rate: timing_number(timings, &["prompt_per_second", "prompt_eval_rate"]),
        eval_count: timing_number(timings, &["predicted_n", "eval_count"])
            .map(|value| value.round() as u64),
        eval_duration_ms: timing_number(timings, &["predicted_ms", "eval_duration_ms"]),
        eval_rate: timing_number(timings, &["predicted_per_second", "eval_rate"]),
    }
}

fn timing_number(value: &serde_json::Value, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|key| {
        value.get(*key).and_then(|entry| {
            entry
                .as_f64()
                .or_else(|| entry.as_str()?.parse::<f64>().ok())
        })
    })
}

fn llama_perf_metrics(text: &str) -> RuntimeMetrics {
    let mut metrics = RuntimeMetrics::default();
    for line in text.lines().map(str::trim) {
        if !line.starts_with("llama_perf_") {
            continue;
        }
        if line.contains("load time") {
            metrics.load_duration_ms = metric_ms_after_equals(line);
        } else if line.contains("prompt eval time") {
            metrics.prompt_eval_duration_ms = metric_ms_after_equals(line);
            metrics.prompt_eval_count = metric_count(line, "tokens");
            metrics.prompt_eval_rate = metric_rate(line);
        } else if line.contains("eval time") {
            metrics.eval_duration_ms = metric_ms_after_equals(line);
            metrics.eval_count =
                metric_count(line, "runs").or_else(|| metric_count(line, "tokens"));
            metrics.eval_rate = metric_rate(line);
        } else if line.contains("total time") {
            metrics.total_duration_ms = metric_ms_after_equals(line);
        }
    }
    metrics
}

fn metric_ms_after_equals(line: &str) -> Option<f64> {
    let after_equals = line.split_once('=')?.1;
    first_number(after_equals)
}

fn metric_count(line: &str, label: &str) -> Option<u64> {
    let slash = line.rfind('/')?;
    let after_slash = &line[slash + 1..];
    let before_label = after_slash.split(label).next()?;
    first_number(before_label).map(|value| value.round() as u64)
}

fn metric_rate(line: &str) -> Option<f64> {
    let before_rate = line.split("tokens per second").next()?;
    let open = before_rate.rfind('(')?;
    first_number(&before_rate[open + 1..])
}

fn first_number(text: &str) -> Option<f64> {
    text.split(|ch: char| !(ch.is_ascii_digit() || ch == '.'))
        .find(|part| !part.is_empty())
        .and_then(|part| part.parse::<f64>().ok())
}

/// llama-cli's context window must hold the prompt tokens *and* the
/// requested generation budget, or it overflows and aborts mid-run. Estimate
/// prompt tokens conservatively (~3 chars/token) and size the context to fit
/// prompt + max_tokens plus headroom, clamped to a range that stays cheap on
/// low-VRAM cards.
fn context_size_for(prompt: &str, max_tokens: u32) -> u32 {
    let prompt_token_estimate = (prompt.chars().count() as u32 / 3).max(32);
    let needed = prompt_token_estimate
        .saturating_add(max_tokens)
        .saturating_add(256);
    needed.clamp(1024, 4096)
}

fn first_actionable_stderr_line(stderr: &str) -> &str {
    stderr
        .lines()
        .find(|line| {
            let trimmed = line.trim();
            !trimmed.is_empty()
                && !trimmed.starts_with("ggml_cuda_init:")
                && !trimmed.starts_with("  Device ")
        })
        .or_else(|| stderr.lines().find(|line| !line.trim().is_empty()))
        .unwrap_or("no stderr")
}

pub fn probe_worker_health(
    model_dir: &Path,
    model_name: Option<&str>,
    backend: Backend,
    contributed_cluster: Option<&crate::storage::ContributedCluster>,
) -> WorkerHealthReport {
    // A node contributing an already-running cluster has no MundusX runtime to
    // inspect: its health is whether that endpoint still answers. Taken as an
    // argument so the probe stays a function of its inputs.
    if let Some(cluster) = contributed_cluster {
        return contributed_cluster_health(model_dir, backend, cluster);
    }

    let mut notes = Vec::new();
    let mut model_path = None;
    let mut llama_cli_available = false;
    let mut llama_server_available = false;
    let mut mlx_available = false;
    let mlx_runtime_url = (backend == Backend::M)
        .then(configured_mlx_server_url)
        .flatten();
    let llama_runtime_url = configured_llama_server_url();
    let persistent_runtime_url = if backend == Backend::M {
        mlx_runtime_url
            .clone()
            .or_else(|| llama_runtime_url.clone())
    } else {
        llama_runtime_url.clone()
    };
    let persistent_mlx_warm = mlx_runtime_url
        .as_deref()
        .map(mlx_server_health_ok)
        .unwrap_or(false);
    let persistent_llama_warm = llama_runtime_url
        .as_deref()
        .map(llama_server_health_ok)
        .unwrap_or(false);
    let persistent_runtime_warm = persistent_mlx_warm || persistent_llama_warm;
    let mut blas_device_available = false;
    let power_state = probe_power_state();
    let cuda = probe_cuda_diagnostics();
    let vllm_url = configured_vllm_url();
    let vllm_ready = vllm_url.as_deref().map(vllm_health_ok).unwrap_or(false);

    if !matches!(backend, Backend::Vllm | Backend::M) {
        match resolve_model_path(model_dir, model_name) {
            Ok(path) => model_path = Some(path.display().to_string()),
            Err(error) => notes.push(format!("model cache missing: {error}")),
        }
    }

    match backend {
        Backend::Cuda => match probe_llama_cli_available() {
            Ok(()) => llama_cli_available = true,
            Err(error) => notes.push(error),
        },
        Backend::Vulkan => match probe_vulkan_device() {
            Ok(name) => {
                llama_cli_available = true;
                notes.push(format!("Vulkan device detected: {name}"));
            }
            Err(error) => notes.push(error),
        },
        Backend::Vllm => {
            if let Some(url) = vllm_url.as_deref() {
                if vllm_ready {
                    notes.push(format!("vLLM runtime is healthy at {url}"));
                } else {
                    notes.push(format!("vLLM runtime is configured but unhealthy at {url}"));
                }
            } else {
                notes.push(
                    "vLLM runtime URL is not configured; set OPENGPU_VLLM_URL to the local OpenAI-compatible endpoint"
                        .to_string(),
                );
            }
            if model_name.is_none() {
                notes.push("vLLM requires an explicit active model".to_string());
            }
        }
        _ => match probe_llama_cli_devices() {
            Ok(stdout) => {
                llama_cli_available = true;
                blas_device_available = stdout.lines().any(|line| line.contains("BLAS"));
                if !blas_device_available {
                    notes.push("BLAS device not listed by llama-cli".to_string());
                }
            }
            Err(error) => notes.push(error),
        },
    }
    if backend == Backend::M {
        match probe_mlx_available() {
            Ok(path) => {
                mlx_available = true;
                notes.push(format!("MLX runtime is available at {}", path.display()));
            }
            Err(error) => notes.push(format!("MLX runtime unavailable: {error}")),
        }
        if let Some(url) = mlx_runtime_url.as_deref() {
            if persistent_mlx_warm {
                notes.push(format!("persistent MLX runtime is warm at {url}"));
            } else {
                notes.push(format!(
                    "persistent MLX runtime is configured but unhealthy at {url}"
                ));
            }
        } else if mlx_available {
            notes.push("persistent MLX runtime is available but not currently warm".to_string());
        }
        if persistent_llama_warm {
            notes.push("persistent llama-server fallback runtime is warm".to_string());
        }
    }
    if backend != Backend::M || (!mlx_available && !persistent_mlx_warm) || persistent_llama_warm {
        match probe_llama_server_executable() {
            Ok(_) => {
                llama_server_available = true;
                if persistent_llama_warm {
                    notes.push("persistent llama-server runtime is warm".to_string());
                } else {
                    notes.push(
                        "persistent llama-server runtime is available but not currently warm"
                            .to_string(),
                    );
                }
            }
            Err(error) => notes.push(format!("persistent runtime unavailable: {error}")),
        }
    }

    if backend == Backend::Cuda {
        notes.extend(cuda.notes.clone());
        if !cuda.device_available {
            notes.push(
                "CUDA device not detected; node will stay unavailable for CUDA jobs".to_string(),
            );
        }
    }

    let local_runtime_available = llama_cli_available || persistent_runtime_warm || mlx_available;
    let healthy = if backend == Backend::Cuda {
        model_path.is_some()
            && local_runtime_available
            && cuda.device_available
            && cuda.driver_available
    } else if backend == Backend::Vulkan {
        model_path.is_some() && llama_cli_available
    } else if backend == Backend::Vllm {
        cfg!(target_os = "linux") && model_name.is_some() && vllm_ready
    } else if backend == Backend::M {
        model_name.is_some() && (mlx_available || persistent_runtime_warm)
    } else if backend == Backend::Auto {
        model_path.is_some() && local_runtime_available
    } else {
        model_path.is_some()
            && local_runtime_available
            && (blas_device_available || persistent_runtime_warm)
    };

    let runtime_mode = match backend {
        Backend::Cuda => "cuda",
        Backend::Vulkan => "vulkan",
        Backend::Vllm => "vllm",
        Backend::M if mlx_available || persistent_mlx_warm => "mlx",
        _ => "blas",
    };
    let runtime_mode = runtime_mode.to_string();
    let runtime_kind = if backend == Backend::Vllm && vllm_ready {
        "persistent-warm".to_string()
    } else if backend == Backend::Vllm {
        "persistent-unavailable".to_string()
    } else if persistent_runtime_warm {
        "persistent-warm".to_string()
    } else if llama_server_available {
        "persistent-unavailable".to_string()
    } else {
        "batch".to_string()
    };
    let supported_runtime_modes = if healthy {
        // This field is a control-plane scheduling contract, not a low-level
        // engine list. MLX still satisfies local execution; the exact engine is
        // reported separately through runtime_mode/runtime_preference.
        vec!["local".to_string()]
    } else {
        Vec::new()
    };

    WorkerHealthReport {
        healthy,
        model_dir: model_dir.display().to_string(),
        model_name: model_name.map(|name| name.to_string()),
        model_path,
        llama_cli_available,
        llama_server_available,
        persistent_runtime_warm,
        persistent_runtime_url,
        runtime_kind,
        runtime_preference: if backend == Backend::M {
            Some(
                if mlx_available || persistent_mlx_warm {
                    "mlx"
                } else {
                    "llama-metal"
                }
                .to_string(),
            )
        } else {
            None
        },
        fallback_runtime: if backend == Backend::M {
            Some(
                if mlx_available || persistent_mlx_warm {
                    "llama-metal"
                } else {
                    "mlx"
                }
                .to_string(),
            )
        } else {
            None
        },
        mlx_available,
        blas_device_available,
        cuda_device_available: cuda.device_available,
        cuda_driver_available: cuda.driver_available,
        cuda_device_name: cuda.device_name,
        cuda_memory_mb: cuda.memory_mb,
        cuda_low_vram_profile: cuda.low_vram_profile,
        power_source: power_state.source,
        on_battery: power_state.on_battery,
        battery_percent: power_state.battery_percent,
        runtime_mode,
        parallel_slots: 1,
        supported_runtime_modes,
        streaming_supported: backend == Backend::Vllm && vllm_ready,
        capabilities: Default::default(),
        checked_at: now_unix_seconds(),
        notes,
    }
}

/// Health report for a node serving an already-running local cluster.
fn contributed_cluster_health(
    model_dir: &Path,
    backend: Backend,
    cluster: &crate::storage::ContributedCluster,
) -> WorkerHealthReport {
    let power_state = probe_power_state();
    let cuda = probe_cuda_diagnostics();
    let reachable = cluster_endpoint_healthy(&cluster.base_url);
    let model_name = cluster
        .model
        .clone()
        .or_else(|| cluster.models.first().cloned());

    let mut notes = Vec::new();
    if reachable {
        notes.push(format!(
            "contributed {} cluster is answering at {}",
            cluster.kind, cluster.base_url
        ));
    } else {
        notes.push(format!(
            "contributed {} cluster at {} is not answering; start it or run `opengpu cluster forget`",
            cluster.kind, cluster.base_url
        ));
    }
    if model_name.is_none() {
        notes.push("contributed cluster advertises no model".to_string());
    }
    notes.push(format!(
        "contribution cap does not gate a contributed cluster; detected backend is {}",
        backend.as_str()
    ));

    // The cluster owns its own memory and batching, so the contribution cap does
    // not gate it and no local model file is expected.
    let healthy = reachable && model_name.is_some();

    WorkerHealthReport {
        healthy,
        model_dir: model_dir.display().to_string(),
        model_name,
        model_path: None,
        llama_cli_available: false,
        llama_server_available: false,
        persistent_runtime_warm: reachable,
        persistent_runtime_url: Some(cluster.base_url.clone()),
        runtime_kind: if reachable {
            "contributed-cluster".to_string()
        } else {
            "contributed-cluster-unreachable".to_string()
        },
        runtime_preference: Some(cluster.kind.clone()),
        fallback_runtime: None,
        mlx_available: false,
        blas_device_available: false,
        cuda_device_available: cuda.device_available,
        cuda_driver_available: cuda.driver_available,
        cuda_device_name: cuda.device_name,
        cuda_memory_mb: cuda.memory_mb,
        cuda_low_vram_profile: cuda.low_vram_profile,
        power_source: power_state.source,
        on_battery: power_state.on_battery,
        battery_percent: power_state.battery_percent,
        runtime_mode: "contributed-cluster".to_string(),
        parallel_slots: 1,
        supported_runtime_modes: if healthy {
            vec!["local".to_string()]
        } else {
            Vec::new()
        },
        streaming_supported: healthy
            && matches!(
                cluster.kind.trim().to_ascii_lowercase().as_str(),
                "vllm" | "openai"
            ),
        capabilities: Default::default(),
        checked_at: now_unix_seconds(),
        notes,
    }
}

pub fn probe_worker_policy(
    health: &WorkerHealthReport,
    contribution_percent: u8,
) -> WorkerPolicyReport {
    let mut notes = Vec::new();
    let mut reason = None;
    let mut allowed = true;
    let recommended_max_contribution_percent;

    if !health.healthy {
        allowed = false;
        reason = Some("worker health is degraded".to_string());
        notes.push("runner health check failed".to_string());
    }

    if contribution_percent == 0 {
        allowed = false;
        reason = Some("contribution percent is unset".to_string());
        notes.push("set a contribution cap before enabling jobs".to_string());
    }

    if health.on_battery {
        recommended_max_contribution_percent = 20;
        if contribution_percent > 20 {
            allowed = false;
            reason = Some("battery power requires contribution percent <= 20".to_string());
            notes.push("plug in the Mac or lower the cap to 20% or less".to_string());
        }
        if let Some(percent) = health.battery_percent {
            if percent <= 20 {
                allowed = false;
                reason = Some("battery too low for active inference".to_string());
                notes.push("battery level is too low to start work safely".to_string());
            }
        }
    } else {
        recommended_max_contribution_percent = 100;
    }

    WorkerPolicyReport {
        allowed,
        reason,
        power_source: health.power_source.clone(),
        on_battery: health.on_battery,
        battery_percent: health.battery_percent,
        recommended_max_contribution_percent,
        checked_at: now_unix_seconds(),
        notes,
    }
}

fn extract_llama_response(prompt: &str, transcript: &str) -> String {
    let generated_before_logs = transcript
        .lines()
        .map(str::trim)
        .take_while(|line| !is_llama_diagnostic_line(line))
        .filter(|line| !line.is_empty() && !line.starts_with('>'))
        .collect::<Vec<_>>();
    if !generated_before_logs.is_empty() {
        return generated_before_logs.join("\n");
    }

    let mut seen_prompt = false;
    let mut lines = Vec::new();

    for line in transcript.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if trimmed == format!("> {prompt}") {
            seen_prompt = true;
            continue;
        }

        if seen_prompt {
            if trimmed.starts_with('[')
                || trimmed.starts_with("Exiting")
                || trimmed.starts_with("available commands")
            {
                break;
            }

            if trimmed.starts_with('>') {
                break;
            }

            lines.push(trimmed.to_string());
        }
    }

    if lines.is_empty() {
        transcript
            .lines()
            .rev()
            .map(str::trim)
            .find(|line| !line.is_empty() && !line.starts_with('[') && !line.starts_with('>'))
            .unwrap_or(transcript)
            .to_string()
    } else {
        lines.join("\n")
    }
}

fn is_llama_diagnostic_line(line: &str) -> bool {
    line.starts_with("ggml_")
        || line.starts_with("build:")
        || line.starts_with("main:")
        || line.starts_with("llama_")
        || line.starts_with("common_")
        || line.starts_with("print_info:")
        || line.starts_with("load:")
        || line.starts_with("load_tensors:")
        || line.starts_with("system_info:")
        || line.starts_with("sampler ")
        || line.starts_with("sampler\t")
        || line.starts_with("sampler params:")
        || line.starts_with("sampler chain:")
        || line.starts_with("generate:")
        || line.starts_with("llama_perf_")
        || line.starts_with("  Device ")
        || line.starts_with("Available devices:")
        || line.starts_with('\t')
}

fn run_llama_request(
    request: &WorkerLaunchRequest,
    backend: Backend,
) -> Result<WorkerLaunchResponse, String> {
    let model_dir = env::var_os("OPENGPU_MODEL_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("/tmp"))
                .join(".opengpu/models")
        });
    let speakai = is_speakai_request(request);
    let model_name = preferred_speakai_model_name(&model_dir, request.model.as_deref(), speakai)
        .unwrap_or_else(|| "active".to_string());

    let base_system_prompt = effective_system_prompt(request);
    let max_tokens = request.max_tokens.unwrap_or(16).max(1);
    let temperature = request.temperature.unwrap_or(0.2).max(0.0);
    let top_p = request.top_p.unwrap_or(0.9).clamp(0.0, 1.0);
    let seed = request.seed.unwrap_or(42);
    let attempts = if speakai { SPEAKAI_MAX_ATTEMPTS } else { 1 };

    if backend == Backend::M {
        let mut last_validation_error = None;
        let mut mlx_started = false;
        for attempt in 0..attempts {
            let system_prompt = speakai_retry_system_prompt(
                base_system_prompt,
                attempt,
                last_validation_error.as_deref(),
            );
            let prompt = if system_prompt.is_empty() {
                request.prompt.clone()
            } else {
                format!("System:\n{system_prompt}\n\nUser:\n{}", request.prompt)
            };
            let attempt_temperature = if attempt == 0 { temperature } else { 0.0 };
            let attempt_seed = seed.saturating_add(attempt as u64);
            let warm_result = configured_mlx_server_url()
                .as_deref()
                .filter(|url| mlx_server_health_ok(url))
                .map(|url| {
                    run_openai_compatible_completion(
                        url,
                        &model_name,
                        &system_prompt,
                        &request.prompt,
                        max_tokens,
                        attempt_temperature,
                        top_p,
                        attempt_seed,
                        speakai,
                    )
                    .map(|generated| {
                        (
                            generated,
                            "persistent-warm-mlx".to_string(),
                            RuntimeMetrics::default(),
                        )
                    })
                });
            let generated = match warm_result {
                Some(Ok(result)) => Ok(result),
                Some(Err(_)) | None => {
                    run_mlx_command(&model_name, &prompt, max_tokens, attempt_temperature)
                }
            };
            match generated {
                Ok((generated, runtime_mode, metrics)) => {
                    mlx_started = true;
                    match normalize_generated_output(request, &generated) {
                        Ok(generated) => {
                            let runtime_metrics = metrics.to_output_fragment().unwrap_or_default();
                            let validation = speakai
                                .then(|| {
                                    format!(
                                        "; structured_output=validated; generation_attempts={}",
                                        attempt + 1
                                    )
                                })
                                .unwrap_or_default();
                            return Ok(WorkerLaunchResponse {
                                job_id: request.job_id.clone(),
                                worker_id: format!("worker-{}", uuid::Uuid::new_v4().simple()),
                                status: "completed".to_string(),
                                output: format!(
                                    "mlx-lm mode={runtime_mode}; model={model_name}; max_tokens={max_tokens}; temperature={temperature}{runtime_metrics}{validation}; response={generated}",
                                ),
                                error: None,
                                backend,
                                node_id: request.node_id.clone(),
                                model: Some(model_name.clone()),
                                runtime_mode: Some(runtime_mode),
                            });
                        }
                        Err(error) => last_validation_error = Some(error),
                    }
                }
                Err(_) if !mlx_started => break,
                Err(error) => return Err(error),
            }
        }
        if mlx_started {
            return Err(format!(
                "SpeakAI generation failed schema validation after {attempts} attempts: {}",
                last_validation_error.unwrap_or_else(|| "invalid structured output".to_string())
            ));
        }
    }

    let model_path =
        resolve_model_path(&model_dir, Some(&model_name)).map_err(|error| error.to_string())?;
    let mut last_validation_error = None;
    for attempt in 0..attempts {
        let system_prompt = speakai_retry_system_prompt(
            base_system_prompt,
            attempt,
            last_validation_error.as_deref(),
        );
        let prompt = if system_prompt.is_empty() {
            request.prompt.clone()
        } else {
            format!("System:\n{system_prompt}\n\nUser:\n{}", request.prompt)
        };
        let attempt_temperature = if speakai && attempt > 0 {
            0.0
        } else {
            temperature
        };
        let attempt_seed = seed.saturating_add(attempt as u64);
        let warm_result = configured_llama_server_url()
            .as_deref()
            .filter(|url| llama_server_health_ok(url))
            .map(|url| {
                run_llama_server_completion(
                    url,
                    &system_prompt,
                    &request.prompt,
                    backend,
                    max_tokens,
                    attempt_temperature,
                    top_p,
                    attempt_seed,
                    speakai,
                )
            });
        let (generated, runtime_mode, metrics) = match warm_result {
            Some(Ok(result)) => result,
            Some(Err(_)) | None => run_llama_command(
                &model_path,
                &prompt,
                backend,
                max_tokens,
                attempt_temperature,
                top_p,
                attempt_seed,
                speakai.then_some(SPEAKAI_JSON_GRAMMAR),
            )?,
        };
        match normalize_generated_output(request, &generated) {
            Ok(generated) => {
                let runtime_metrics = metrics.to_output_fragment().unwrap_or_default();
                let validation = speakai
                    .then(|| {
                        format!(
                            "; structured_output=validated; generation_attempts={}",
                            attempt + 1
                        )
                    })
                    .unwrap_or_default();
                return Ok(WorkerLaunchResponse {
                    job_id: request.job_id.clone(),
                    worker_id: format!("worker-{}", uuid::Uuid::new_v4().simple()),
                    status: "completed".to_string(),
                    output: format!(
                        "llama.cpp mode={runtime_mode}; model={model_name}; path={}; max_tokens={max_tokens}; temperature={attempt_temperature}; top_p={top_p}; seed={attempt_seed}{runtime_metrics}{validation}; response={generated}",
                        model_path.display(),
                    ),
                    error: None,
                    backend,
                    node_id: request.node_id.clone(),
                    model: Some(model_name.clone()),
                    runtime_mode: Some(runtime_mode),
                });
            }
            Err(error) => last_validation_error = Some(error),
        }
    }

    Err(format!(
        "SpeakAI generation failed schema validation after {attempts} attempts: {}",
        last_validation_error.unwrap_or_else(|| "invalid structured output".to_string())
    ))
}

fn run_vllm_request(request: &WorkerLaunchRequest) -> Result<WorkerLaunchResponse, String> {
    if !cfg!(target_os = "linux") {
        return Err("vLLM execution is supported only on Linux nodes".to_string());
    }
    let url = configured_vllm_url()
        .ok_or_else(|| "vLLM runtime URL is not configured; set OPENGPU_VLLM_URL".to_string())?;
    if !vllm_health_ok(&url) {
        return Err(format!("vLLM runtime is not healthy at {url}"));
    }
    let model = request
        .model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "vLLM jobs require an explicit model".to_string())?;
    let max_tokens = request.max_tokens.unwrap_or(16).max(1);
    let temperature = request.temperature.unwrap_or(0.2).max(0.0);
    let top_p = request.top_p.unwrap_or(0.9).clamp(0.0, 1.0);
    let seed = request.seed.unwrap_or(42);
    let speakai = is_speakai_request(request);
    let attempts = if speakai { SPEAKAI_MAX_ATTEMPTS } else { 1 };
    let base_system_prompt = effective_system_prompt(request);
    let mut last_validation_error = None;
    for attempt in 0..attempts {
        let system_prompt = speakai_retry_system_prompt(
            base_system_prompt,
            attempt,
            last_validation_error.as_deref(),
        );
        let attempt_temperature = if speakai && attempt > 0 {
            0.0
        } else {
            temperature
        };
        let attempt_seed = seed.saturating_add(attempt as u64);
        let generated = run_openai_compatible_completion(
            &url,
            model,
            &system_prompt,
            &request.prompt,
            max_tokens,
            attempt_temperature,
            top_p,
            attempt_seed,
            speakai,
        )?;
        match normalize_generated_output(request, &generated) {
            Ok(generated) => {
                let validation = speakai
                    .then(|| {
                        format!(
                            "; structured_output=validated; generation_attempts={}",
                            attempt + 1
                        )
                    })
                    .unwrap_or_default();
                return Ok(WorkerLaunchResponse {
                    job_id: request.job_id.clone(),
                    worker_id: format!("worker-{}", uuid::Uuid::new_v4().simple()),
                    status: "completed".to_string(),
                    output: format!(
                        "vLLM mode=persistent-warm; model={model}; max_tokens={max_tokens}; temperature={attempt_temperature}; top_p={top_p}; seed={attempt_seed}{validation}; response={generated}"
                    ),
                    error: None,
                    backend: Backend::Vllm,
                    node_id: request.node_id.clone(),
                    model: Some(model.to_string()),
                    runtime_mode: Some("vllm".to_string()),
                });
            }
            Err(error) => last_validation_error = Some(error),
        }
    }
    Err(format!(
        "SpeakAI generation failed schema validation after {attempts} attempts: {}",
        last_validation_error.unwrap_or_else(|| "invalid structured output".to_string())
    ))
}

/// Runs a job against the contributed cluster's OpenAI-compatible endpoint.
fn run_contributed_cluster_request(
    request: &WorkerLaunchRequest,
    cluster: &crate::storage::ContributedCluster,
) -> Result<WorkerLaunchResponse, String> {
    if !cluster_endpoint_healthy(&cluster.base_url) {
        return Err(format!(
            "contributed {} cluster is not answering at {}",
            cluster.kind, cluster.base_url
        ));
    }

    // The cluster decides what it serves, so a job asking for a different model
    // is refused rather than silently answered by the wrong one.
    let advertised = cluster
        .model
        .clone()
        .or_else(|| cluster.models.first().cloned())
        .ok_or_else(|| "contributed cluster advertises no model".to_string())?;
    if let Some(requested) = request
        .model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if requested != advertised && !cluster.models.iter().any(|entry| entry == requested) {
            return Err(format!(
                "contributed cluster serves {advertised}, not {requested}"
            ));
        }
    }
    let speakai = is_speakai_request(request);
    let model = request
        .model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            speakai.then(|| {
                cluster
                    .models
                    .iter()
                    .find(|name| {
                        let name = name.to_ascii_lowercase();
                        name.contains("qwen") || name.contains("phi-4") || name.contains("gemma")
                    })
                    .map(String::as_str)
                    .unwrap_or(&advertised)
            })
        })
        .unwrap_or(&advertised);

    let max_tokens = request.max_tokens.unwrap_or(16).max(1);
    let temperature = request.temperature.unwrap_or(0.2).max(0.0);
    let top_p = request.top_p.unwrap_or(0.9).clamp(0.0, 1.0);
    let seed = request.seed.unwrap_or(42);
    let attempts = if speakai { SPEAKAI_MAX_ATTEMPTS } else { 1 };
    let base_system_prompt = effective_system_prompt(request);
    let mut last_validation_error = None;
    for attempt in 0..attempts {
        let system_prompt = speakai_retry_system_prompt(
            base_system_prompt,
            attempt,
            last_validation_error.as_deref(),
        );
        let attempt_temperature = if speakai && attempt > 0 {
            0.0
        } else {
            temperature
        };
        let attempt_seed = seed.saturating_add(attempt as u64);
        let generated = run_openai_compatible_completion(
            cluster.base_url.trim_end_matches('/'),
            model,
            &system_prompt,
            &request.prompt,
            max_tokens,
            attempt_temperature,
            top_p,
            attempt_seed,
            speakai,
        )?;
        match normalize_generated_output(request, &generated) {
            Ok(generated) => {
                let validation = speakai
                    .then(|| {
                        format!(
                            "; structured_output=validated; generation_attempts={}",
                            attempt + 1
                        )
                    })
                    .unwrap_or_default();
                return Ok(WorkerLaunchResponse {
                    job_id: request.job_id.clone(),
                    worker_id: format!("worker-{}", uuid::Uuid::new_v4().simple()),
                    status: "completed".to_string(),
                    output: format!(
                        "contributed-cluster kind={}; endpoint={}; model={model}; max_tokens={max_tokens}; temperature={attempt_temperature}; top_p={top_p}; seed={attempt_seed}{validation}; response={generated}",
                        cluster.kind, cluster.base_url
                    ),
                    error: None,
                    backend: resolved_backend(request.backend),
                    node_id: request.node_id.clone(),
                    model: Some(model.to_string()),
                    runtime_mode: Some("contributed-cluster".to_string()),
                });
            }
            Err(error) => last_validation_error = Some(error),
        }
    }
    Err(format!(
        "SpeakAI generation failed schema validation after {attempts} attempts: {}",
        last_validation_error.unwrap_or_else(|| "invalid structured output".to_string())
    ))
}

fn execute_request(request: &WorkerLaunchRequest) -> WorkerLaunchResponse {
    execute_request_with_cluster(request, contributed_cluster().as_ref())
}

/// Job execution with the contributed cluster supplied explicitly, so tests do
/// not depend on whatever config happens to be on the developer's machine.
fn execute_request_with_cluster(
    request: &WorkerLaunchRequest,
    contributed_cluster: Option<&crate::storage::ContributedCluster>,
) -> WorkerLaunchResponse {
    let backend = resolved_backend(request.backend);

    // A contributed cluster serves every job for this node, whatever the
    // machine's own backend would have been.
    if let Some(cluster) = contributed_cluster {
        return match run_contributed_cluster_request(request, cluster) {
            Ok(response) => response,
            Err(error) => WorkerLaunchResponse {
                job_id: request.job_id.clone(),
                worker_id: format!("worker-{}", uuid::Uuid::new_v4().simple()),
                status: "failed".to_string(),
                output: String::new(),
                error: Some(error),
                backend,
                node_id: request.node_id.clone(),
                model: request.model.clone(),
                runtime_mode: Some("contributed-cluster".to_string()),
            },
        };
    }

    if backend == Backend::Vllm {
        return match run_vllm_request(request) {
            Ok(response) => response,
            Err(error) => WorkerLaunchResponse {
                job_id: request.job_id.clone(),
                worker_id: format!("worker-{}", uuid::Uuid::new_v4().simple()),
                status: "failed".to_string(),
                output: String::new(),
                error: Some(error),
                backend,
                node_id: request.node_id.clone(),
                model: request.model.clone(),
                runtime_mode: Some("vllm".to_string()),
            },
        };
    }
    if matches!(
        backend,
        Backend::Auto | Backend::M | Backend::Cuda | Backend::Vulkan
    ) {
        return match run_llama_request(request, backend) {
            Ok(response) => response,
            Err(error) => WorkerLaunchResponse {
                job_id: request.job_id.clone(),
                worker_id: format!("worker-{}", uuid::Uuid::new_v4().simple()),
                status: "failed".to_string(),
                output: String::new(),
                error: Some(error),
                backend,
                node_id: request.node_id.clone(),
                model: request.model.clone(),
                runtime_mode: Some(backend.as_str().to_string()),
            },
        };
    }

    WorkerLaunchResponse {
        job_id: request.job_id.clone(),
        worker_id: format!("worker-{}", uuid::Uuid::new_v4().simple()),
        status: "failed".to_string(),
        output: String::new(),
        error: Some(format!(
            "backend {backend} is not enabled in this worker runtime"
        )),
        backend,
        node_id: request.node_id.clone(),
        model: request.model.clone(),
        runtime_mode: Some(backend.as_str().to_string()),
    }
}

pub fn worker_main(cli: WorkerCli) {
    let live_stream = cli.stream;
    let request = WorkerLaunchRequest {
        job_id: cli.job_id,
        node_id: cli.node_id,
        backend: cli.backend,
        stream: live_stream,
        prompt: cli.prompt,
        model: cli.model,
        mode: cli.mode,
        system_prompt: cli.system_prompt,
        max_tokens: cli.max_tokens,
        temperature: cli.temperature,
        top_p: cli.top_p,
        seed: cli.seed,
    };

    let response = if live_stream {
        let (delta_tx, delta_rx) = mpsc::channel::<String>();
        let printer = thread::spawn(move || {
            let stderr = io::stderr();
            let mut stderr = stderr.lock();
            while let Ok(delta) = delta_rx.recv() {
                let _ = writeln!(stderr, "{STREAM_DELTA_PREFIX}{}", hex::encode(delta));
                let _ = stderr.flush();
            }
        });
        let response = with_stream_delta_sender(Some(delta_tx), || execute_request(&request));
        let _ = printer.join();
        response
    } else {
        execute_request(&request)
    };

    if cli.json {
        if let Err(error) = emit_json(&response) {
            eprintln!("failed to print worker json: {error}");
            std::process::exit(1);
        }
        return;
    }

    println!("workerId: {}", response.worker_id);
    println!("jobId: {}", response.job_id);
    println!("nodeId: {}", response.node_id);
    println!("backend: {}", response.backend);
    println!("status: {}", response.status);
    println!("{}", output_summary_line(&response.output));
}

fn output_summary_line(output: &str) -> String {
    let chars = output.chars().count();
    let first_line = output
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(str::trim)
        .unwrap_or("");
    let preview: String = first_line.chars().take(160).collect();
    if chars > preview.chars().count() {
        format!("output: {chars} chars; preview: {preview}...")
    } else {
        format!("output: {chars} chars; preview: {preview}")
    }
}

/// Path to re-exec for the worker subprocess.
///
/// Linux reports a replaced binary as `/path/to/exe (deleted)`, which is what
/// `env::current_exe` returns after an upgrade replaced the file underneath a
/// running agent. Spawning that path fails with ENOENT, so strip the marker and
/// use the current file at the same location when one exists.
fn worker_executable() -> Result<PathBuf, String> {
    let exe = env::current_exe().map_err(|error| error.to_string())?;
    if exe.exists() {
        return Ok(exe);
    }

    let raw = exe.to_string_lossy();
    if let Some(path) = raw.strip_suffix(" (deleted)") {
        let replaced = PathBuf::from(path);
        if replaced.exists() {
            return Ok(replaced);
        }
    }

    Err(format!(
        "the node agent binary at {} was replaced or removed while running; restart `opengpu start` to pick up the new build",
        raw.trim_end_matches(" (deleted)")
    ))
}

pub fn launch_worker_with_stream(
    request: &WorkerLaunchRequest,
    model_dir: &Path,
    delta_sender: Option<mpsc::Sender<String>>,
) -> Result<WorkerLaunchResponse, String> {
    // A contributed cluster is served over HTTP, so there is nothing to isolate
    // in a subprocess and no reason to depend on re-execing this binary.
    if let Some(cluster) = contributed_cluster() {
        return Ok(with_stream_delta_sender(delta_sender, || {
            execute_request_with_cluster(request, Some(&cluster))
        }));
    }

    let mut command = Command::new(worker_executable()?);
    command
        .env("OPENGPU_MODEL_DIR", model_dir)
        .arg("worker")
        .arg("--job-id")
        .arg(&request.job_id)
        .arg("--node-id")
        .arg(&request.node_id)
        .arg("--prompt")
        .arg(&request.prompt)
        .arg("--backend")
        .arg(request.backend.as_str());

    if let Some(model) = request.model.as_ref() {
        command.arg("--model").arg(model);
    }
    if let Some(mode) = request.mode.as_ref() {
        command.arg("--mode").arg(mode);
    }
    if let Some(system_prompt) = request.system_prompt.as_ref() {
        command.arg("--system-prompt").arg(system_prompt);
    }
    if let Some(max_tokens) = request.max_tokens {
        command.arg("--max-tokens").arg(max_tokens.to_string());
    }
    if let Some(temperature) = request.temperature {
        command.arg("--temperature").arg(temperature.to_string());
    }
    if let Some(top_p) = request.top_p {
        command.arg("--top-p").arg(top_p.to_string());
    }
    if let Some(seed) = request.seed {
        command.arg("--seed").arg(seed.to_string());
    }
    if request.stream {
        command.arg("--stream");
    }

    let mut child = command
        .arg("--json")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("failed to launch worker: {error}"))?;

    let mut stdout_pipe = child.stdout.take().expect("worker stdout piped");
    let mut stderr_pipe = child.stderr.take().expect("worker stderr piped");
    let stdout_reader = thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout_pipe.read_to_end(&mut buf);
        buf
    });
    let stderr_reader = thread::spawn(move || {
        let mut diagnostics = Vec::new();
        let mut reader = BufReader::new(&mut stderr_pipe);
        let mut line = Vec::new();
        loop {
            line.clear();
            match reader.read_until(b'\n', &mut line) {
                Ok(0) => break,
                Ok(_) => {
                    let text = String::from_utf8_lossy(&line);
                    if let Some(encoded) = text.trim().strip_prefix(STREAM_DELTA_PREFIX) {
                        if let Ok(bytes) = hex::decode(encoded) {
                            if let Ok(delta) = String::from_utf8(bytes) {
                                if let Some(sender) = delta_sender.as_ref() {
                                    let _ = sender.send(delta);
                                }
                                continue;
                            }
                        }
                    }
                    diagnostics.extend_from_slice(&line);
                }
                Err(_) => break,
            }
        }
        diagnostics
    });

    let deadline = Instant::now() + worker_timeout();
    let status = loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("failed to poll worker: {error}"))?
        {
            break status;
        }
        if Instant::now() >= deadline {
            // Kill the whole tree, not just the direct child: the worker
            // subprocess spawns llama-cli as its own child, and on Windows
            // that grandchild can inherit our stdout/stderr pipe handles.
            // A single-process kill leaves llama-cli running as an orphan
            // holding the pipe open, which would hang stdout_reader/
            // stderr_reader forever waiting for EOF that never comes — so
            // deliberately skip joining them here and let them unwind on
            // their own once the tree-kill closes every handle.
            kill_process_tree(child.id());
            let _ = child.wait();
            return Err(format!(
                "worker timed out after {}s and was terminated",
                worker_timeout().as_secs()
            ));
        }
        thread::sleep(Duration::from_millis(200));
    };

    let stdout = stdout_reader.join().unwrap_or_default();
    let stderr = stderr_reader.join().unwrap_or_default();

    if !status.success() {
        let stderr = String::from_utf8_lossy(&stderr);
        return Err(format!(
            "worker exited {} — {}",
            status.code().unwrap_or(-1),
            stderr.lines().next().unwrap_or("no stderr")
        ));
    }

    let stdout = String::from_utf8(stdout).map_err(|error| error.to_string())?;
    serde_json::from_str(stdout.trim()).map_err(|error| error.to_string())
}

pub fn launch_worker(
    request: &WorkerLaunchRequest,
    model_dir: &Path,
) -> Result<WorkerLaunchResponse, String> {
    launch_worker_with_stream(request, model_dir, None)
}

fn worker_timeout() -> Duration {
    std::env::var("OPENGPU_WORKER_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .map(Duration::from_secs)
        .unwrap_or_else(|| Duration::from_secs(300))
}

#[cfg(windows)]
fn kill_process_tree(pid: u32) {
    let _ = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(not(windows))]
fn kill_process_tree(pid: u32) {
    let _ = Command::new("pkill")
        .args(["-9", "-P", &pid.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = Command::new("kill")
        .args(["-9", &pid.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(test)]
mod tests {
    /// Job execution with no contributed cluster, so these tests never depend on
    /// a `config.json` that happens to exist on the machine running them.
    fn execute_without_cluster(
        request: &super::WorkerLaunchRequest,
    ) -> super::WorkerLaunchResponse {
        super::execute_request_with_cluster(request, None)
    }

    use super::*;
    use std::io::Cursor;
    use std::io::Write;
    use std::net::TcpListener;
    use std::sync::{Mutex, OnceLock};

    #[test]
    fn parses_openai_sse_and_relays_safe_progressive_prefix() {
        let content =
            "Streaming sends several useful pieces while retaining a small validation tail.";
        let first = &content[..24];
        let second = &content[24..52];
        let third = &content[52..];
        let body = format!(
            "data: {{\"choices\":[{{\"delta\":{{\"role\":\"assistant\",\"content\":\"\"}},\"finish_reason\":null}}]}}\n\n\
             data: {{\"choices\":[{{\"delta\":{{\"content\":{}}},\"finish_reason\":null}}]}}\n\n\
             data: {{\"choices\":[{{\"delta\":{{\"content\":{}}},\"finish_reason\":null}}]}}\n\n\
             data: {{\"choices\":[{{\"delta\":{{\"content\":{}}},\"finish_reason\":\"stop\"}}]}}\n\n\
             data: [DONE]\n\n",
            serde_json::to_string(first).unwrap(),
            serde_json::to_string(second).unwrap(),
            serde_json::to_string(third).unwrap(),
        );
        let (sender, receiver) = mpsc::channel();
        let parsed = with_stream_delta_sender(Some(sender), || {
            parse_openai_stream(Cursor::new(body)).expect("stream parses")
        });
        let relayed = receiver.try_iter().collect::<String>();

        assert_eq!(parsed, content);
        assert!(!relayed.is_empty());
        assert!(content.starts_with(&relayed));
        assert!(content.chars().count() - relayed.chars().count() >= STREAM_TAIL_HOLD_CHARS);
    }

    #[test]
    fn context_size_covers_prompt_and_generation_budget() {
        let prompt = "Give me a detailed history of honda from its origins to today.";
        let max_tokens = 768;
        let context = context_size_for(prompt, max_tokens);
        let prompt_token_estimate = (prompt.chars().count() as u32 / 3).max(32);
        assert!(
            context >= prompt_token_estimate + max_tokens,
            "context {context} must fit prompt (~{prompt_token_estimate} tokens) plus max_tokens {max_tokens}"
        );
    }

    #[test]
    fn context_size_has_a_floor_for_short_prompts() {
        assert_eq!(context_size_for("hi", 4), 1024);
    }

    #[test]
    fn context_size_is_capped_for_low_vram_cards() {
        assert_eq!(context_size_for("hi", 100_000), 4096);
    }

    #[test]
    fn stop_persistent_runtime_without_state_is_noop() {
        with_temp_runtime_home(|_| {
            let stopped = stop_persistent_runtime_from_state().expect("stop runtime");
            assert!(stopped.is_none());
        });
    }

    #[test]
    fn stop_persistent_runtime_removes_recorded_state() {
        with_temp_runtime_home(|home| {
            let state_path = home.join("persistent-runtime.json");
            fs::write(
                &state_path,
                serde_json::json!({
                    "pid": 4_294_967_295_u32,
                    "url": "http://127.0.0.1:8789",
                    "environment_variable": "OPENGPU_LLAMA_SERVER_URL",
                    "container_name": null,
                    "updated_at": "123",
                })
                .to_string(),
            )
            .expect("runtime state");

            let stopped = stop_persistent_runtime_from_state()
                .expect("stop runtime")
                .expect("runtime state should be returned");

            assert_eq!(stopped.pid, 4_294_967_295_u32);
            assert_eq!(stopped.url, "http://127.0.0.1:8789");
            assert!(!state_path.exists());
        });
    }

    fn with_temp_runtime_home(test: impl FnOnce(&Path)) {
        static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        let _guard = ENV_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();
        let temp_dir = std::env::temp_dir().join(format!(
            "opengpu-agent-runtime-paths-{}",
            uuid::Uuid::new_v4().simple()
        ));
        fs::create_dir_all(&temp_dir).expect("temp dir");
        let previous_home = env::var_os("OPENGPU_HOME");
        let previous_paths = env::var_os("OPENGPU_TRUSTED_RUNTIME_PATHS");
        let previous_server_url = env::var_os("OPENGPU_LLAMA_SERVER_URL");
        let previous_mlx_url = env::var_os("OPENGPU_MLX_SERVER_URL");
        let previous_vllm_url = env::var_os("OPENGPU_VLLM_URL");
        env::set_var("OPENGPU_HOME", &temp_dir);
        env::set_var(
            "OPENGPU_TRUSTED_RUNTIME_PATHS",
            temp_dir.join("trusted-runtime-paths.json"),
        );
        env::remove_var("OPENGPU_LLAMA_SERVER_URL");
        env::remove_var("OPENGPU_MLX_SERVER_URL");
        env::remove_var("OPENGPU_VLLM_URL");

        test(&temp_dir);

        match previous_home {
            Some(value) => env::set_var("OPENGPU_HOME", value),
            None => env::remove_var("OPENGPU_HOME"),
        }
        match previous_paths {
            Some(value) => env::set_var("OPENGPU_TRUSTED_RUNTIME_PATHS", value),
            None => env::remove_var("OPENGPU_TRUSTED_RUNTIME_PATHS"),
        }
        match previous_server_url {
            Some(value) => env::set_var("OPENGPU_LLAMA_SERVER_URL", value),
            None => env::remove_var("OPENGPU_LLAMA_SERVER_URL"),
        }
        match previous_mlx_url {
            Some(value) => env::set_var("OPENGPU_MLX_SERVER_URL", value),
            None => env::remove_var("OPENGPU_MLX_SERVER_URL"),
        }
        match previous_vllm_url {
            Some(value) => env::set_var("OPENGPU_VLLM_URL", value),
            None => env::remove_var("OPENGPU_VLLM_URL"),
        }
        let _ = fs::remove_dir_all(temp_dir);
    }

    fn write_trusted_paths(home: &Path, paths: TrustedRuntimePaths) {
        fs::write(
            home.join("trusted-runtime-paths.json"),
            serde_json::to_string_pretty(&paths).expect("trusted paths json"),
        )
        .expect("trusted paths");
    }

    fn start_mock_llama_server(completion: &'static str, expected_requests: usize) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock llama server");
        let addr = listener.local_addr().expect("mock llama server addr");
        std::thread::spawn(move || {
            for _ in 0..expected_requests {
                let (mut stream, _) = listener.accept().expect("mock llama accept");
                let mut request_bytes = Vec::new();
                let mut header_end = None;
                let mut content_length = 0_usize;
                loop {
                    let mut buffer = [0_u8; 1024];
                    let size = stream.read(&mut buffer).expect("mock llama read");
                    if size == 0 {
                        break;
                    }
                    request_bytes.extend_from_slice(&buffer[..size]);
                    if header_end.is_none() {
                        header_end = request_bytes
                            .windows(4)
                            .position(|window| window == b"\r\n\r\n")
                            .map(|index| index + 4);
                        if let Some(end) = header_end {
                            let headers = String::from_utf8_lossy(&request_bytes[..end]);
                            content_length = headers
                                .lines()
                                .find_map(|line| {
                                    let (name, value) = line.split_once(':')?;
                                    if name.eq_ignore_ascii_case("content-length") {
                                        value.trim().parse::<usize>().ok()
                                    } else {
                                        None
                                    }
                                })
                                .unwrap_or(0);
                        }
                    }
                    if let Some(end) = header_end {
                        if request_bytes.len() >= end + content_length {
                            break;
                        }
                    }
                }
                let request = String::from_utf8_lossy(&request_bytes);
                let body = if request.starts_with("GET /health")
                    || request.starts_with("GET /v1/models")
                {
                    "{}".to_string()
                } else {
                    assert!(
                        request.starts_with("POST /v1/chat/completions"),
                        "unexpected request: {request}"
                    );
                    assert!(
                        request.contains("\"messages\"")
                            && request.contains("\"role\":\"system\"")
                            && request.contains("\"role\":\"user\"")
                            && request.contains("\"max_tokens\""),
                        "chat completion request should include messages and token budget: {request}"
                    );
                    serde_json::json!({
                        "choices": [{
                            "message": { "role": "assistant", "content": completion }
                        }]
                    })
                    .to_string()
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("mock llama write");
            }
        });

        format!("http://{addr}")
    }

    fn start_mock_speakai_server(completions: Vec<&'static str>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind SpeakAI mock server");
        let addr = listener.local_addr().expect("SpeakAI mock server addr");
        std::thread::spawn(move || {
            let mut completion_index = 0_usize;
            let expected_requests = completions.len() * 2;
            for _ in 0..expected_requests {
                let (mut stream, _) = listener.accept().expect("SpeakAI mock accept");
                let mut request_bytes = Vec::new();
                let mut header_end = None;
                let mut content_length = 0_usize;
                loop {
                    let mut buffer = [0_u8; 2048];
                    let size = stream.read(&mut buffer).expect("SpeakAI mock read");
                    if size == 0 {
                        break;
                    }
                    request_bytes.extend_from_slice(&buffer[..size]);
                    if header_end.is_none() {
                        header_end = request_bytes
                            .windows(4)
                            .position(|window| window == b"\r\n\r\n")
                            .map(|index| index + 4);
                        if let Some(end) = header_end {
                            let headers = String::from_utf8_lossy(&request_bytes[..end]);
                            content_length = headers
                                .lines()
                                .find_map(|line| {
                                    let (name, value) = line.split_once(':')?;
                                    name.eq_ignore_ascii_case("content-length")
                                        .then(|| value.trim().parse::<usize>().ok())
                                        .flatten()
                                })
                                .unwrap_or(0);
                        }
                    }
                    if header_end.is_some_and(|end| request_bytes.len() >= end + content_length) {
                        break;
                    }
                }

                let request = String::from_utf8_lossy(&request_bytes);
                let body = if request.starts_with("GET /health")
                    || request.starts_with("GET /v1/models")
                {
                    "{}".to_string()
                } else {
                    assert!(request.starts_with("POST /v1/chat/completions"));
                    assert!(request.contains("\"response_format\""));
                    assert!(request.contains("\"type\":\"json_schema\""));
                    let completion = completions[completion_index];
                    completion_index += 1;
                    serde_json::json!({
                        "choices": [{
                            "message": { "role": "assistant", "content": completion }
                        }]
                    })
                    .to_string()
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(), body
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("SpeakAI mock write");
            }
        });
        format!("http://{addr}")
    }

    fn speakai_request(model: Option<&str>) -> WorkerLaunchRequest {
        WorkerLaunchRequest {
            job_id: "speakai-job".to_string(),
            node_id: "node-1".to_string(),
            backend: Backend::M,
            stream: false,
            prompt: "Heute ist das Wetter schön.".to_string(),
            model: model.map(str::to_string),
            mode: Some("speakai".to_string()),
            system_prompt: None,
            max_tokens: Some(512),
            temperature: Some(0.2),
            top_p: Some(0.9),
            seed: Some(42),
        }
    }

    #[test]
    fn speakai_prompt_requires_a_direct_utterance_translation() {
        assert!(SPEAKAI_SYSTEM_PROMPT.contains(
            "summary must be only a direct, natural English translation"
        ));
        assert!(SPEAKAI_SYSTEM_PROMPT.contains("never an explanation"));
    }

    #[test]
    fn normalizes_speakai_required_fields_and_reply_contract() {
        let output = r#"```json
        {"speechAct":"OBSERVATION","questionType":"other","topic":" weather ","summary":" nice day ","extra":"drop me","replies":[
          {"strategy":"wrong","purpose":"wrong","text":"Stimmt!","meaning":"That's true!"},
          {"strategy":"wrong","purpose":"wrong","text":"Magst du Sonne?","meaning":"Do you like sunshine?"},
          {"strategy":"wrong","purpose":"wrong","text":"Morgen regnet es vielleicht.","meaning":"It may rain tomorrow."}
        ]}
        ```"#;

        let normalized = validate_and_normalize_speakai_output(output).expect("valid SpeakAI");
        let value: serde_json::Value = serde_json::from_str(&normalized).expect("normalized JSON");
        assert_eq!(value["speechAct"], "observation");
        assert_eq!(value["topic"], "weather");
        assert_eq!(value["replies"][0]["strategy"], "acknowledge");
        assert_eq!(value["replies"][0]["purpose"], "ACKNOWLEDGE");
        assert!(value.get("questionType").is_none());
        assert!(value.get("extra").is_none());
    }

    #[test]
    fn rejects_malformed_or_incomplete_speakai_output() {
        assert!(validate_and_normalize_speakai_output("{not json}").is_err());
        assert!(validate_and_normalize_speakai_output(
            r#"{"speechAct":"greeting","topic":"hello","summary":"greeting","replies":[]}"#
        )
        .is_err());
    }

    #[test]
    fn rejects_an_explanatory_summary_instead_of_a_direct_translation() {
        let output = r#"{"speechAct":"question","questionType":"other","topic":"Wellbeing","summary":"The speaker asks how the listener is.","replies":[{"strategy":"direct","purpose":"ANSWER","text":"Mir geht es gut.","meaning":"I am well."},{"strategy":"continue","purpose":"ANSWER_AND_EXPLORE","text":"Mir geht es gut, und Ihnen?","meaning":"I am well, and you?"},{"strategy":"boundary","purpose":"DECLINE_POLITELY","text":"Darüber möchte ich nicht sprechen.","meaning":"I do not want to talk about that."}]}"#;

        let error = validate_and_normalize_speakai_output(output)
            .expect_err("explanatory summary must be retried");
        assert!(error.contains("directly translate"));
    }

    #[test]
    fn normalizes_conflicting_greeting_purposes_to_the_owned_contract() {
        let output = r#"{"speechAct":"greeting","topic":"Greeting","summary":"The speaker greets the listener.","replies":[{"strategy":"friendly","purpose":"GREET","text":"Hallo, guten Tag!","meaning":"Hello, good day!"},{"strategy":"question","purpose":"ASK_WELLBEING","text":"Wie geht es Ihnen?","meaning":"How are you?"},{"strategy":"polite","purpose":"WELCOME","text":"Schön, Sie zu sehen.","meaning":"Nice to see you."}]}"#;

        let normalized = validate_and_normalize_speakai_output(output).expect("valid greeting");
        let value: serde_json::Value = serde_json::from_str(&normalized).expect("normalized JSON");
        assert_eq!(value["replies"][0]["strategy"], "direct");
        assert_eq!(value["replies"][0]["purpose"], "RETURN_GREETING");
        assert_eq!(value["replies"][1]["purpose"], "START_CONVERSATION");
        assert_eq!(value["replies"][2]["purpose"], "WARM_VARIATION");
        assert_eq!(value["replies"][0]["text"], "Hallo, guten Tag!");
    }

    #[test]
    fn speakai_retries_invalid_json_and_completes_only_after_validation() {
        with_temp_runtime_home(|home| {
            let model_dir = home.join("models");
            let model_cache = model_dir.join("qwen");
            fs::create_dir_all(&model_cache).expect("model dir");
            fs::write(model_cache.join("model.gguf"), b"model").expect("model file");
            write_trusted_paths(
                home,
                TrustedRuntimePaths {
                    llama_cli: Some(TrustedExecutable {
                        path: home.join("missing-llama-cli").display().to_string(),
                        sha256: None,
                    }),
                    llama_server: None,
                    nvidia_smi: None,
                },
            );
            let previous_model_dir = env::var_os("OPENGPU_MODEL_DIR");
            env::set_var("OPENGPU_MODEL_DIR", &model_dir);
            let valid = r#"{"speechAct":"greeting","topic":"hello","summary":"A greeting","replies":[{"strategy":"bad","purpose":"bad","text":"Hallo!","meaning":"Hello!"},{"strategy":"bad","purpose":"bad","text":"Wie geht es dir?","meaning":"How are you?"},{"strategy":"bad","purpose":"bad","text":"Schön, dich zu sehen.","meaning":"Nice to see you."}]}"#;
            env::set_var(
                "OPENGPU_MLX_SERVER_URL",
                start_mock_speakai_server(vec!["{broken", valid]),
            );

            let response = execute_without_cluster(&speakai_request(Some(
                "mlx-community/Qwen2.5-3B-Instruct-4bit",
            )));

            match previous_model_dir {
                Some(value) => env::set_var("OPENGPU_MODEL_DIR", value),
                None => env::remove_var("OPENGPU_MODEL_DIR"),
            }
            assert_eq!(response.status, "completed");
            assert_eq!(
                response.runtime_mode.as_deref(),
                Some("persistent-warm-mlx")
            );
            assert!(response.output.contains("structured_output=validated"));
            assert!(response.output.contains("generation_attempts=2"));
            assert!(response.output.contains("\"strategy\":\"direct\""));
            assert!(!response.output.contains("\"strategy\":\"bad\""));
        });
    }

    #[test]
    fn speakai_never_completes_after_all_internal_attempts_fail() {
        with_temp_runtime_home(|home| {
            let model_dir = home.join("models");
            let model_cache = model_dir.join("qwen");
            fs::create_dir_all(&model_cache).expect("model dir");
            fs::write(model_cache.join("model.gguf"), b"model").expect("model file");
            let previous_model_dir = env::var_os("OPENGPU_MODEL_DIR");
            env::set_var("OPENGPU_MODEL_DIR", &model_dir);
            env::set_var(
                "OPENGPU_LLAMA_SERVER_URL",
                start_mock_speakai_server(vec!["{broken", "not json", "{}"]),
            );

            let response = execute_without_cluster(&speakai_request(Some("qwen")));

            match previous_model_dir {
                Some(value) => env::set_var("OPENGPU_MODEL_DIR", value),
                None => env::remove_var("OPENGPU_MODEL_DIR"),
            }
            assert_eq!(response.status, "failed");
            assert!(response.output.is_empty());
            assert!(response
                .error
                .as_deref()
                .is_some_and(|error| error.contains("after 3 attempts")));
        });
    }

    #[test]
    fn speakai_prefers_an_installed_structured_output_model() {
        let model_dir = env::temp_dir().join(format!(
            "speakai-model-preference-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let manifest_dir = model_dir.join(".opengpu");
        fs::create_dir_all(&manifest_dir).expect("manifest dir");
        for (name, active, structured) in [
            ("HuggingFaceTB/SmolLM2-135M-Instruct", true, false),
            ("Qwen/Qwen2.5-3B-Instruct", false, true),
        ] {
            let cache = model_dir.join(sanitize_model_name(name));
            fs::create_dir_all(&cache).expect("cache dir");
            fs::write(cache.join("model.gguf"), b"model").expect("model");
            fs::write(
                manifest_dir.join(format!("{}.json", sanitize_model_name(name))),
                serde_json::json!({
                    "name": name,
                    "active": active,
                    "file_name": "model.gguf",
                    "supports_structured_output": structured,
                    "specialties": if structured { vec!["speakai"] } else { Vec::<&str>::new() }
                })
                .to_string(),
            )
            .expect("manifest");
        }

        assert_eq!(
            preferred_speakai_model_name(&model_dir, None, true).as_deref(),
            Some("Qwen/Qwen2.5-3B-Instruct")
        );
        let _ = fs::remove_dir_all(model_dir);
    }

    #[test]
    fn parses_low_vram_cuda_device_from_nvidia_smi() {
        let diagnostics = parse_nvidia_smi_query("NVIDIA GeForce GTX 1050 Ti, 4096\n");

        assert!(diagnostics.device_available);
        assert!(diagnostics.driver_available);
        assert_eq!(
            diagnostics.device_name.as_deref(),
            Some("NVIDIA GeForce GTX 1050 Ti")
        );
        assert_eq!(diagnostics.memory_mb, Some(4096));
        assert!(diagnostics.low_vram_profile);
        assert!(diagnostics
            .notes
            .iter()
            .any(|note| note.contains("low-VRAM profile")));
    }

    #[test]
    fn detects_gb10_when_nvidia_smi_reports_unified_memory() {
        let diagnostics = parse_nvidia_smi_query("NVIDIA GB10, [N/A]\n");

        assert!(diagnostics.device_available);
        assert!(diagnostics.driver_available);
        assert_eq!(diagnostics.device_name.as_deref(), Some("NVIDIA GB10"));
        assert_eq!(diagnostics.memory_mb, None);
        assert!(!diagnostics.low_vram_profile);
        assert!(diagnostics
            .notes
            .iter()
            .any(|note| note.contains("unified system memory")));
    }

    #[test]
    fn vllm_slots_use_cap_applied_unified_memory() {
        assert_eq!(
            recommended_parallel_slots(Backend::Vllm, None, Some(124_000), 65, Some("Qwen2.5-32B"),),
            1
        );
        assert_eq!(
            recommended_parallel_slots(Backend::Vllm, None, Some(124_000), 65, Some("Qwen2.5-14B"),),
            2
        );
    }

    #[test]
    fn parses_vllm_startup_progress_for_any_shard_count() {
        assert_eq!(
            vllm_startup_progress(
                "Loading safetensors checkpoint shards:  31% Completed | 5/16 [03:27<07:40]"
            )
            .as_deref(),
            Some("loading checkpoint shards 5/16 (31%)")
        );
        assert_eq!(
            vllm_startup_progress("Downloading (incomplete total...):  64%| 35.8G/56.0G")
                .as_deref(),
            Some("downloading model files (64%)")
        );
        assert_eq!(
            vllm_startup_progress("INFO: Application startup complete.").as_deref(),
            Some("ready (100%)")
        );
    }

    #[test]
    fn low_vram_cuda_nodes_disable_parallelism() {
        assert_eq!(
            recommended_parallel_slots(Backend::Cuda, Some(4096), None, 80, Some("Qwen2.5-3B"),),
            1
        );
    }

    #[test]
    fn rtx_5090_advertises_four_small_model_slots_at_eighty_percent() {
        assert_eq!(
            recommended_parallel_slots(Backend::Cuda, Some(32_768), None, 80, Some("Qwen2.5-3B"),),
            4
        );
        assert_eq!(
            recommended_parallel_slots(Backend::Cuda, Some(32_768), None, 80, Some("Qwen2.5-7B"),),
            3
        );
        assert_eq!(
            recommended_parallel_slots(Backend::Cuda, Some(32_768), None, 80, Some("Qwen2.5-32B"),),
            1
        );
    }

    #[test]
    fn mlx_slots_scale_with_cap_applied_unified_memory() {
        assert_eq!(
            recommended_parallel_slots(Backend::M, None, Some(16_384), 80, Some("Qwen2.5-3B")),
            1
        );
        assert_eq!(
            recommended_parallel_slots(Backend::M, None, Some(32_768), 80, Some("Qwen2.5-3B")),
            2
        );
        assert_eq!(
            recommended_parallel_slots(Backend::M, None, Some(64_000), 80, Some("Qwen2.5-3B")),
            3
        );
        assert_eq!(
            recommended_parallel_slots(Backend::M, None, Some(128_000), 80, Some("Qwen2.5-3B")),
            4
        );
    }

    #[test]
    fn mlx_large_models_reduce_parallel_slots() {
        assert_eq!(
            recommended_parallel_slots(Backend::M, None, Some(128_000), 80, Some("Qwen2.5-14B")),
            2
        );
        assert_eq!(
            recommended_parallel_slots(Backend::M, None, Some(128_000), 80, Some("Qwen2.5-32B")),
            1
        );
    }

    #[test]
    fn chooses_largest_cuda_device_from_nvidia_smi() {
        let diagnostics =
            parse_nvidia_smi_query("NVIDIA GeForce GTX 1050 Ti, 4096\nNVIDIA RTX 4090, 24564\n");

        assert_eq!(diagnostics.device_name.as_deref(), Some("NVIDIA RTX 4090"));
        assert_eq!(diagnostics.memory_mb, Some(24564));
        assert!(!diagnostics.low_vram_profile);
    }

    #[test]
    fn resolves_imported_model_source_path_from_manifest() {
        let temp_dir = std::env::temp_dir().join(format!(
            "opengpu-agent-imported-model-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let manifest_dir = temp_dir.join(".opengpu");
        fs::create_dir_all(&manifest_dir).expect("manifest dir");
        let source_path = temp_dir.join("external-q4_k_m.gguf");
        fs::write(&source_path, b"model").expect("model file");
        fs::write(
            manifest_dir.join("external.json"),
            serde_json::json!({
                "name": "external",
                "active": true,
                "cached_at": "1",
                "model_dir": temp_dir,
                "source_path": source_path,
            })
            .to_string(),
        )
        .expect("manifest");

        let resolved = resolve_model_path(&temp_dir, Some("external")).expect("resolve model");

        assert_eq!(resolved, source_path);
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn resolves_downloaded_model_from_cli_cache_directory() {
        let temp_dir = std::env::temp_dir().join(format!(
            "opengpu-agent-downloaded-model-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let manifest_dir = temp_dir.join(".opengpu");
        fs::create_dir_all(&manifest_dir).expect("manifest dir");
        let cache_dir = temp_dir.join("qwen_qwen2_5-0_5b-instruct");
        fs::create_dir_all(&cache_dir).expect("cache dir");
        let model_path = cache_dir.join("qwen2.5-0.5b-instruct-q5_k_m.gguf");
        fs::write(&model_path, b"model").expect("model file");
        fs::write(
            manifest_dir.join("qwen_qwen2_5-0_5b-instruct.json"),
            serde_json::json!({
                "name": "Qwen/Qwen2.5-0.5B-Instruct",
                "active": true,
                "cached_at": "1",
                "model_dir": temp_dir,
            })
            .to_string(),
        )
        .expect("manifest");

        let resolved = resolve_model_path(&temp_dir, Some("Qwen/Qwen2.5-0.5B-Instruct"))
            .expect("resolve downloaded model");

        assert_eq!(resolved, model_path);
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn missing_requested_model_does_not_fallback_to_other_cache() {
        let temp_dir = std::env::temp_dir().join(format!(
            "opengpu-agent-missing-requested-model-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let manifest_dir = temp_dir.join(".opengpu");
        fs::create_dir_all(&manifest_dir).expect("manifest dir");
        let smol_dir = temp_dir.join("huggingfacetb_smollm2-135m-instruct");
        fs::create_dir_all(&smol_dir).expect("smol dir");
        fs::write(smol_dir.join("SmolLM2-135M-Instruct.Q4_K_M.gguf"), b"model")
            .expect("smol model");
        fs::write(
            manifest_dir.join("qwen_qwen2_5-3b-instruct.json"),
            serde_json::json!({
                "name": "Qwen/Qwen2.5-3B-Instruct",
                "active": true,
                "cached_at": "1",
                "model_dir": temp_dir,
            })
            .to_string(),
        )
        .expect("manifest");

        let error = resolve_model_path(&temp_dir, Some("Qwen/Qwen2.5-3B-Instruct"))
            .expect_err("missing requested model should not fallback");

        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn extracts_generated_text_before_llama_diagnostics() {
        let transcript = " 4. What is the answer to\r\n\r\nggml_cuda_init: found 1 CUDA devices:\r\nbuild: 4500\r\nllama_perf_context_print: total time = 1 ms\r\n";

        let generated = extract_llama_response("The answer to 2+2 is", transcript);

        assert_eq!(generated, "4. What is the answer to");
    }

    #[test]
    fn parses_llama_perf_metrics() {
        let metrics = llama_perf_metrics(
            "llama_perf_context_print:        load time =     123.45 ms\n\
             llama_perf_context_print: prompt eval time =      50.00 ms /    10 tokens (  200.00 tokens per second)\n\
             llama_perf_context_print:        eval time =     100.00 ms /     5 runs   (   50.00 tokens per second)\n\
             llama_perf_context_print:       total time =     275.00 ms /    15 tokens",
        );

        assert_eq!(metrics.load_duration_ms, Some(123.45));
        assert_eq!(metrics.prompt_eval_count, Some(10));
        assert_eq!(metrics.prompt_eval_duration_ms, Some(50.0));
        assert_eq!(metrics.prompt_eval_rate, Some(200.0));
        assert_eq!(metrics.eval_count, Some(5));
        assert_eq!(metrics.eval_duration_ms, Some(100.0));
        assert_eq!(metrics.eval_rate, Some(50.0));
        assert_eq!(metrics.total_duration_ms, Some(275.0));
    }

    #[test]
    fn parses_llama_server_timing_metrics() {
        let value = serde_json::json!({
            "content": "hello",
            "timings": {
                "prompt_n": 12,
                "prompt_ms": 60.0,
                "prompt_per_second": 200.0,
                "predicted_n": 8,
                "predicted_ms": 160.0,
                "predicted_per_second": 50.0
            }
        });

        let metrics = llama_server_metrics(&value);

        assert_eq!(metrics.prompt_eval_count, Some(12));
        assert_eq!(metrics.prompt_eval_duration_ms, Some(60.0));
        assert_eq!(metrics.prompt_eval_rate, Some(200.0));
        assert_eq!(metrics.eval_count, Some(8));
        assert_eq!(metrics.eval_duration_ms, Some(160.0));
        assert_eq!(metrics.eval_rate, Some(50.0));
    }

    #[test]
    fn parses_vulkan_device_from_llama_device_list() {
        let devices = "Available devices:\n  Vulkan0: Intel(R) Iris(R) Xe Graphics (8192 MiB, 7168 MiB free)\n";
        assert_eq!(
            vulkan_device_name(devices).as_deref(),
            Some("Intel(R) Iris(R) Xe Graphics (8192 MiB, 7168 MiB free)")
        );
        assert_eq!(vulkan_device_name("BLAS: CPU"), None);
    }

    #[test]
    fn vulkan_worker_uses_llama_runtime_path() {
        let response = execute_without_cluster(&WorkerLaunchRequest {
            job_id: "job-vulkan".to_string(),
            node_id: "node-1".to_string(),
            backend: Backend::Vulkan,
            stream: false,
            prompt: "hello".to_string(),
            model: None,
            mode: None,
            system_prompt: None,
            max_tokens: Some(4),
            temperature: Some(0.2),
            top_p: Some(0.9),
            seed: Some(42),
        });
        assert_eq!(response.backend, Backend::Vulkan);
        assert_eq!(response.runtime_mode.as_deref(), Some("vulkan"));
        assert!(!response
            .error
            .as_deref()
            .unwrap_or("")
            .contains("not enabled"));
    }

    #[test]
    fn cuda_worker_uses_llama_runtime_instead_of_m_only_rejection() {
        with_temp_runtime_home(|home| {
            let model_dir = home.join("models");
            let model_cache = model_dir.join("qwen");
            fs::create_dir_all(&model_cache).expect("model dir");
            fs::write(model_cache.join("model.gguf"), b"model").expect("model file");
            write_trusted_paths(
                home,
                TrustedRuntimePaths {
                    llama_cli: Some(TrustedExecutable {
                        path: home.join("missing-llama-cli").display().to_string(),
                        sha256: None,
                    }),
                    llama_server: None,
                    nvidia_smi: None,
                },
            );
            let previous_model_dir = env::var_os("OPENGPU_MODEL_DIR");
            env::set_var("OPENGPU_MODEL_DIR", &model_dir);

            let response = execute_without_cluster(&WorkerLaunchRequest {
                job_id: "job-1".to_string(),
                node_id: "node-1".to_string(),
                backend: Backend::Cuda,
                stream: false,
                prompt: "hello".to_string(),
                model: Some("qwen".to_string()),
                mode: None,
                system_prompt: None,
                max_tokens: Some(4),
                temperature: Some(0.2),
                top_p: Some(0.9),
                seed: Some(42),
            });

            match previous_model_dir {
                Some(value) => env::set_var("OPENGPU_MODEL_DIR", value),
                None => env::remove_var("OPENGPU_MODEL_DIR"),
            }

            assert_eq!(response.backend, Backend::Cuda);
            assert_eq!(response.runtime_mode.as_deref(), Some("cuda"));
            let error = response.error.expect("missing llama-cli error");
            assert!(error.contains("missing llama-cli"));
            assert!(!error.contains("Mac M-only worker"));
        });
    }

    #[test]
    fn cuda_health_requires_llama_runtime_before_advertising_local_support() {
        with_temp_runtime_home(|home| {
            let model_dir = home.join("models");
            let model_cache = model_dir.join("qwen");
            fs::create_dir_all(&model_cache).expect("model dir");
            fs::write(model_cache.join("model.gguf"), b"model").expect("model file");
            write_trusted_paths(
                home,
                TrustedRuntimePaths {
                    llama_cli: Some(TrustedExecutable {
                        path: home.join("missing-llama-cli").display().to_string(),
                        sha256: None,
                    }),
                    llama_server: None,
                    nvidia_smi: None,
                },
            );

            let health = probe_worker_health(&model_dir, Some("qwen"), Backend::Cuda, None);

            assert!(!health.healthy);
            assert!(!health.llama_cli_available);
            assert_eq!(
                health.model_path.as_deref(),
                Some(model_cache.join("model.gguf").to_str().unwrap())
            );
            assert!(health.supported_runtime_modes.is_empty());
            assert!(health
                .notes
                .iter()
                .any(|note| note.contains("missing llama-cli")));
        });
    }

    #[test]
    fn vllm_health_requires_a_configured_runtime_endpoint() {
        with_temp_runtime_home(|home| {
            let model_dir = home.join("models");
            let model_cache = model_dir.join("qwen");
            fs::create_dir_all(&model_cache).expect("model dir");
            fs::write(model_cache.join("model.gguf"), b"model").expect("model file");

            let health = probe_worker_health(&model_dir, Some("qwen"), Backend::Vllm, None);

            assert!(!health.healthy);
            assert_eq!(health.runtime_mode, "vllm");
            assert!(health.supported_runtime_modes.is_empty());
            assert!(health
                .notes
                .iter()
                .any(|note| note.contains("OPENGPU_VLLM_URL")));
        });
    }

    #[test]
    fn vllm_worker_fails_with_clear_runtime_message() {
        with_temp_runtime_home(|_| {
            let response = execute_without_cluster(&WorkerLaunchRequest {
                job_id: "job-vllm".to_string(),
                node_id: "node-1".to_string(),
                backend: Backend::Vllm,
                stream: false,
                prompt: "hello".to_string(),
                model: Some("qwen".to_string()),
                mode: None,
                system_prompt: None,
                max_tokens: Some(4),
                temperature: Some(0.2),
                top_p: Some(0.9),
                seed: Some(42),
            });

            assert_eq!(response.backend, Backend::Vllm);
            assert_eq!(response.status, "failed");
            assert_eq!(response.runtime_mode.as_deref(), Some("vllm"));
            let error = response.error.as_deref().unwrap_or("");
            if cfg!(target_os = "linux") {
                assert!(error.contains("OPENGPU_VLLM_URL"));
            } else {
                assert!(error.contains("Linux nodes"));
            }
        });
    }

    #[test]
    fn worker_executable_resolves_the_current_binary() {
        let resolved = super::worker_executable().expect("current exe");

        assert!(resolved.exists());
        assert!(!resolved.to_string_lossy().ends_with(" (deleted)"));
    }

    #[test]
    fn a_contributed_cluster_job_needs_no_worker_subprocess() {
        // The endpoint is unreachable, so this fails — but it must fail on the
        // HTTP call, not by trying to re-exec the agent binary.
        let cluster = crate::storage::ContributedCluster {
            kind: "llama.cpp".to_string(),
            base_url: "http://127.0.0.1:1".to_string(),
            capacity_class: "server".to_string(),
            models: vec!["m".to_string()],
            model: Some("m".to_string()),
            model_params: None,
            model_bytes: None,
            model_capabilities: Vec::new(),
            model_context_tokens: None,
            memory_utilization: None,
            max_concurrency: None,
            max_num_seqs: None,
            kv_cache_tokens: None,
            supports_tool_calls: false,
            adopted_at: None,
        };

        let response = super::execute_request_with_cluster(
            &WorkerLaunchRequest {
                job_id: "j".to_string(),
                node_id: "n".to_string(),
                prompt: "hello".to_string(),
                model: None,
                mode: None,
                system_prompt: None,
                max_tokens: Some(8),
                temperature: None,
                top_p: None,
                seed: None,
                backend: Backend::Cuda,
                stream: false,
            },
            Some(&cluster),
        );

        assert_eq!(response.status, "failed");
        assert_eq!(
            response.runtime_mode.as_deref(),
            Some("contributed-cluster")
        );
        let error = response.error.expect("error");
        assert!(error.contains("not answering"), "unexpected error: {error}");
        assert!(!error.contains("launch worker"));
    }

    #[test]
    fn truncated_reasoning_output_reports_a_token_budget_problem() {
        // llama.cpp shape when a reasoning model burns the budget before content.
        let body = serde_json::json!({
            "choices": [{
                "finish_reason": "length",
                "message": {"role": "assistant", "content": "", "reasoning_content": "The user is asking"},
            }]
        });

        let message = empty_completion_error(&body);
        assert!(message.contains("spent its whole budget on reasoning"));
        assert!(message.contains("context size"), "message: {message}");
    }

    #[test]
    fn reasoning_truncation_reports_how_many_tokens_were_generated() {
        // GLM-5.2 on a 256-token context: 18 prompt + 238 generated, no content.
        let body = serde_json::json!({
            "choices": [{
                "finish_reason": "length",
                "message": {"content": "", "reasoning_content": "thinking"},
            }],
            "usage": {"prompt_tokens": 18, "completion_tokens": 238, "total_tokens": 256},
        });

        let message = empty_completion_error(&body);

        assert!(
            message.contains("238 tokens generated"),
            "message: {message}"
        );
    }

    #[test]
    fn plain_truncation_and_runtime_errors_are_distinguished() {
        let truncated = serde_json::json!({
            "choices": [{"finish_reason": "length", "message": {"content": ""}}]
        });
        assert!(empty_completion_error(&truncated).contains("generation limit"));

        let failed = serde_json::json!({"error": {"message": "model not loaded"}});
        assert_eq!(
            empty_completion_error(&failed),
            "runtime returned an error: model not loaded"
        );

        let unknown = serde_json::json!({"choices": []});
        assert_eq!(
            empty_completion_error(&unknown),
            "response did not include choices[0].message.content"
        );
    }

    #[test]
    fn vllm_worker_uses_openai_compatible_endpoint() {
        with_temp_runtime_home(|_| {
            let url = start_mock_llama_server("hello from vllm", 2);
            env::set_var("OPENGPU_VLLM_URL", &url);

            let response = execute_without_cluster(&WorkerLaunchRequest {
                job_id: "job-vllm".to_string(),
                node_id: "node-1".to_string(),
                backend: Backend::Vllm,
                stream: false,
                prompt: "hello".to_string(),
                model: Some("Qwen/Qwen3-8B".to_string()),
                mode: None,
                system_prompt: Some("Answer directly.".to_string()),
                max_tokens: Some(4),
                temperature: Some(0.2),
                top_p: Some(0.9),
                seed: Some(42),
            });

            assert_eq!(response.runtime_mode.as_deref(), Some("vllm"));
            assert_eq!(response.model.as_deref(), Some("Qwen/Qwen3-8B"));
            if cfg!(target_os = "linux") {
                assert_eq!(response.status, "completed");
                assert!(response.output.contains("response=hello from vllm"));
            } else {
                assert_eq!(response.status, "failed");
                assert!(response
                    .error
                    .as_deref()
                    .unwrap_or("")
                    .contains("Linux nodes"));
            }
        });
    }

    #[test]
    fn warm_llama_fallback_advertises_m_node_support_without_llama_cli() {
        with_temp_runtime_home(|home| {
            let model_dir = home.join("models");
            let model_cache = model_dir.join("qwen");
            fs::create_dir_all(&model_cache).expect("model dir");
            fs::write(model_cache.join("model.gguf"), b"model").expect("model file");
            let server_runtime = home.join("trusted-llama-server");
            fs::write(&server_runtime, b"trusted server runtime").expect("server runtime");
            write_trusted_paths(
                home,
                TrustedRuntimePaths {
                    llama_cli: None,
                    llama_server: Some(TrustedExecutable {
                        path: server_runtime.display().to_string(),
                        sha256: None,
                    }),
                    nvidia_smi: None,
                },
            );
            let url = start_mock_llama_server("ready", 1);
            env::set_var("OPENGPU_LLAMA_SERVER_URL", url);

            let health = probe_worker_health(&model_dir, Some("qwen"), Backend::M, None);

            assert!(health.healthy);
            assert!(!health.llama_cli_available);
            assert!(health.llama_server_available);
            assert!(health.persistent_runtime_warm);
            assert_eq!(health.runtime_kind, "persistent-warm");
            assert_eq!(health.supported_runtime_modes, vec!["local".to_string()]);
        });
    }

    #[test]
    fn warm_mlx_runtime_advertises_m_node_support() {
        with_temp_runtime_home(|home| {
            let url = start_mock_llama_server("ready", 1);
            env::set_var("OPENGPU_MLX_SERVER_URL", &url);

            let health = probe_worker_health(
                &home.join("models"),
                Some("mlx-community/Qwen2.5-3B-Instruct-4bit"),
                Backend::M,
                None,
            );

            assert!(health.healthy);
            assert!(health.persistent_runtime_warm);
            assert_eq!(health.persistent_runtime_url.as_deref(), Some(url.as_str()));
            assert_eq!(health.runtime_kind, "persistent-warm");
            assert_eq!(health.runtime_mode, "mlx");
            assert_eq!(health.runtime_preference.as_deref(), Some("mlx"));
            assert_eq!(health.supported_runtime_modes, vec!["local".to_string()]);
            assert!(health
                .notes
                .iter()
                .any(|note| note.contains("persistent MLX runtime is warm")));
        });
    }

    #[test]
    fn m_worker_prefers_warm_mlx_server_without_spawning_batch_runtime() {
        with_temp_runtime_home(|_| {
            let url = start_mock_llama_server("hello from warm mlx", 2);
            env::set_var("OPENGPU_MLX_SERVER_URL", url);

            let response = execute_without_cluster(&WorkerLaunchRequest {
                job_id: "job-mlx".to_string(),
                node_id: "node-m".to_string(),
                backend: Backend::M,
                stream: false,
                prompt: "hello".to_string(),
                model: Some("mlx-community/Qwen2.5-3B-Instruct-4bit".to_string()),
                mode: None,
                system_prompt: Some("Answer directly.".to_string()),
                max_tokens: Some(16),
                temperature: Some(0.2),
                top_p: Some(0.9),
                seed: Some(42),
            });

            assert_eq!(response.status, "completed");
            assert_eq!(
                response.runtime_mode.as_deref(),
                Some("persistent-warm-mlx")
            );
            assert!(response.output.contains("response=hello from warm mlx"));
            assert!(response.error.is_none());
        });
    }

    #[test]
    fn execute_request_prefers_warm_llama_server_without_spawning_cli() {
        with_temp_runtime_home(|home| {
            let model_dir = home.join("models");
            let model_cache = model_dir.join("qwen");
            fs::create_dir_all(&model_cache).expect("model dir");
            fs::write(model_cache.join("model.gguf"), b"model").expect("model file");
            write_trusted_paths(
                home,
                TrustedRuntimePaths {
                    llama_cli: Some(TrustedExecutable {
                        path: home.join("missing-llama-cli").display().to_string(),
                        sha256: None,
                    }),
                    llama_server: None,
                    nvidia_smi: None,
                },
            );
            let previous_model_dir = env::var_os("OPENGPU_MODEL_DIR");
            env::set_var("OPENGPU_MODEL_DIR", &model_dir);
            let url = start_mock_llama_server("hello from warm server", 2);
            env::set_var("OPENGPU_LLAMA_SERVER_URL", url);

            let response = execute_without_cluster(&WorkerLaunchRequest {
                job_id: "job-1".to_string(),
                node_id: "node-1".to_string(),
                backend: Backend::M,
                stream: false,
                prompt: "hello".to_string(),
                model: Some("qwen".to_string()),
                mode: None,
                system_prompt: Some("Answer directly.".to_string()),
                max_tokens: Some(4),
                temperature: Some(0.2),
                top_p: Some(0.9),
                seed: Some(42),
            });

            match previous_model_dir {
                Some(value) => env::set_var("OPENGPU_MODEL_DIR", value),
                None => env::remove_var("OPENGPU_MODEL_DIR"),
            }

            assert_eq!(response.status, "completed");
            assert_eq!(
                response.runtime_mode.as_deref(),
                Some("persistent-warm-blas")
            );
            assert!(response.output.contains("response=hello from warm server"));
            assert!(response.error.is_none());
        });
    }

    #[test]
    fn trusted_runtime_uses_pinned_absolute_path() {
        with_temp_runtime_home(|home| {
            let runtime = home.join("trusted-llama-cli");
            fs::write(&runtime, b"trusted runtime").expect("runtime file");
            let digest = sha256_file(&runtime).expect("runtime hash");
            write_trusted_paths(
                home,
                TrustedRuntimePaths {
                    llama_cli: Some(TrustedExecutable {
                        path: runtime.display().to_string(),
                        sha256: Some(digest),
                    }),
                    llama_server: None,
                    nvidia_smi: None,
                },
            );

            let resolved = trusted_runtime_executable("llama-cli").expect("trusted runtime");

            assert_eq!(resolved, runtime);
        });
    }

    #[test]
    fn trusted_runtime_resolves_pinned_llama_server_path() {
        with_temp_runtime_home(|home| {
            let runtime = home.join("trusted-llama-server");
            fs::write(&runtime, b"trusted server runtime").expect("runtime file");
            let digest = sha256_file(&runtime).expect("runtime hash");
            write_trusted_paths(
                home,
                TrustedRuntimePaths {
                    llama_cli: None,
                    llama_server: Some(TrustedExecutable {
                        path: runtime.display().to_string(),
                        sha256: Some(digest),
                    }),
                    nvidia_smi: None,
                },
            );

            let resolved = trusted_runtime_executable("llama-server").expect("trusted runtime");

            assert_eq!(resolved, runtime);
        });
    }

    #[test]
    fn trusted_runtime_resolves_llama_server_beside_pinned_cli() {
        with_temp_runtime_home(|home| {
            let runtime_dir = home.join("runtimes").join("llama");
            fs::create_dir_all(&runtime_dir).expect("runtime dir");
            let cli_runtime = runtime_dir.join(platform_executable_name("llama-cli"));
            let server_runtime = runtime_dir.join(platform_executable_name("llama-server"));
            fs::write(&cli_runtime, b"trusted cli runtime").expect("cli runtime");
            fs::write(&server_runtime, b"trusted server runtime").expect("server runtime");
            let digest = sha256_file(&cli_runtime).expect("cli runtime hash");
            write_trusted_paths(
                home,
                TrustedRuntimePaths {
                    llama_cli: Some(TrustedExecutable {
                        path: cli_runtime.display().to_string(),
                        sha256: Some(digest),
                    }),
                    llama_server: None,
                    nvidia_smi: None,
                },
            );

            let resolved = trusted_runtime_executable("llama-server").expect("trusted sibling");

            assert_eq!(resolved, server_runtime);
        });
    }

    #[test]
    fn trusted_runtime_rejects_llama_server_sibling_when_cli_hash_changed() {
        with_temp_runtime_home(|home| {
            let runtime_dir = home.join("runtimes").join("llama");
            fs::create_dir_all(&runtime_dir).expect("runtime dir");
            let cli_runtime = runtime_dir.join(platform_executable_name("llama-cli"));
            let server_runtime = runtime_dir.join(platform_executable_name("llama-server"));
            fs::write(&cli_runtime, b"trusted cli runtime").expect("cli runtime");
            fs::write(&server_runtime, b"trusted server runtime").expect("server runtime");
            write_trusted_paths(
                home,
                TrustedRuntimePaths {
                    llama_cli: Some(TrustedExecutable {
                        path: cli_runtime.display().to_string(),
                        sha256: Some("not-the-real-hash".to_string()),
                    }),
                    llama_server: None,
                    nvidia_smi: None,
                },
            );

            let error = trusted_runtime_executable("llama-server").expect_err("cli hash must fail");

            assert!(error.contains("untrusted llama-cli"));
        });
    }

    #[test]
    fn trusted_runtime_rejects_hash_change() {
        with_temp_runtime_home(|home| {
            let runtime = home.join("trusted-llama-cli");
            fs::write(&runtime, b"trusted runtime").expect("runtime file");
            write_trusted_paths(
                home,
                TrustedRuntimePaths {
                    llama_cli: Some(TrustedExecutable {
                        path: runtime.display().to_string(),
                        sha256: Some("0".repeat(64)),
                    }),
                    llama_server: None,
                    nvidia_smi: None,
                },
            );

            let error = trusted_runtime_executable("llama-cli").expect_err("hash must fail");

            assert!(error.contains("untrusted llama-cli"));
            assert!(error.contains("hash changed"));
        });
    }

    #[test]
    fn trusted_runtime_rejects_relative_path_lookup() {
        with_temp_runtime_home(|home| {
            write_trusted_paths(
                home,
                TrustedRuntimePaths {
                    llama_cli: Some(TrustedExecutable {
                        path: "llama-cli".to_string(),
                        sha256: None,
                    }),
                    llama_server: None,
                    nvidia_smi: None,
                },
            );

            let error = trusted_runtime_executable("llama-cli").expect_err("relative path");

            assert!(error.contains("pinned runtime path must be absolute"));
        });
    }
}
