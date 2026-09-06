use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

const DEFAULT_CHAT_URL: &str = "https://chat.mundusx.ai";

#[derive(Debug, Serialize)]
struct StartRequest<'a> {
    device_id: &'a str,
    public_key_hex: &'a str,
}

#[derive(Debug, Deserialize)]
struct StartResponse {
    session_id: String,
    bootstrap_secret: String,
    approval_url: String,
    expires_in_seconds: u64,
}

#[derive(Debug, Serialize)]
struct StatusRequest<'a> {
    bootstrap_secret: &'a str,
}

#[derive(Debug, Deserialize)]
struct StatusResponse {
    state: String,
}

pub struct ApprovedBootstrap {
    pub pairing_code: String,
}

pub fn default_chat_url() -> String {
    DEFAULT_CHAT_URL.to_string()
}

pub fn connect(
    chat_url: &str,
    device_id: &str,
    public_key_hex: &str,
    open_browser: bool,
) -> Result<ApprovedBootstrap, String> {
    let base = validate_chat_url(chat_url)?;
    let start: StartResponse = post_json(
        &format!("{base}/api/harness/bootstrap/sessions"),
        &StartRequest {
            device_id,
            public_key_hex,
        },
    )?;
    if start.session_id.is_empty()
        || start.bootstrap_secret.is_empty()
        || start.approval_url.is_empty()
    {
        return Err("HARNESS_BOOTSTRAP_INVALID: Chat returned an incomplete session".to_string());
    }
    validate_approval_url(&base, &start.approval_url)?;
    if open_browser {
        webbrowser::open(&start.approval_url)
            .map_err(|error| format!("HARNESS_BOOTSTRAP_BROWSER_FAILED: {error}"))?;
    } else {
        println!("approvalUrl: {}", start.approval_url);
    }
    println!("harnessBootstrap: waiting for browser approval");
    let deadline = Instant::now() + Duration::from_secs(start.expires_in_seconds.clamp(30, 600));
    let status_url = format!(
        "{base}/api/harness/bootstrap/sessions/{}/status",
        start.session_id
    );
    while Instant::now() < deadline {
        let status: StatusResponse = post_json(
            &status_url,
            &StatusRequest {
                bootstrap_secret: &start.bootstrap_secret,
            },
        )?;
        match status.state.as_str() {
            "approved" => {
                return Ok(ApprovedBootstrap {
                    pairing_code: start.bootstrap_secret,
                });
            }
            "connected" => {
                return Err(
                    "HARNESS_BOOTSTRAP_CONSUMED: this connection was already completed".to_string(),
                )
            }
            "expired" => {
                return Err(
                    "HARNESS_BOOTSTRAP_EXPIRED: start the connection again from Chat".to_string(),
                )
            }
            "pending" => std::thread::sleep(Duration::from_secs(2)),
            _ => {
                return Err(format!(
                    "HARNESS_BOOTSTRAP_INVALID: unknown session state {}",
                    status.state
                ))
            }
        }
    }
    Err("HARNESS_BOOTSTRAP_EXPIRED: browser approval timed out".to_string())
}

fn validate_chat_url(value: &str) -> Result<String, String> {
    let parsed = url::Url::parse(value.trim())
        .map_err(|_| "HARNESS_BOOTSTRAP_URL_INVALID: Chat URL is invalid".to_string())?;
    let local = matches!(parsed.host_str(), Some("localhost" | "127.0.0.1" | "::1"));
    if parsed.scheme() != "https" && !(parsed.scheme() == "http" && local) {
        return Err(
            "HARNESS_BOOTSTRAP_URL_INVALID: Chat URL must use HTTPS (HTTP is allowed only locally)"
                .to_string(),
        );
    }
    Ok(value.trim().trim_end_matches('/').to_string())
}

fn validate_approval_url(chat_url: &str, approval_url: &str) -> Result<(), String> {
    let chat = url::Url::parse(chat_url)
        .map_err(|_| "HARNESS_BOOTSTRAP_URL_INVALID: Chat URL is invalid".to_string())?;
    let approval = url::Url::parse(approval_url)
        .map_err(|_| "HARNESS_BOOTSTRAP_INVALID: approval URL is invalid".to_string())?;
    if chat.scheme() != approval.scheme()
        || chat.host_str() != approval.host_str()
        || chat.port_or_known_default() != approval.port_or_known_default()
    {
        return Err("HARNESS_BOOTSTRAP_INVALID: approval URL does not belong to Chat".to_string());
    }
    Ok(())
}

fn post_json<T: Serialize, R: for<'de> Deserialize<'de>>(
    endpoint: &str,
    body: &T,
) -> Result<R, String> {
    let response = ureq::post(endpoint)
        .set("Accept", "application/json")
        .set("Content-Type", "application/json")
        .send_json(body)
        .map_err(|error| format!("HARNESS_BOOTSTRAP_UNAVAILABLE: {error}"))?;
    response
        .into_json::<R>()
        .map_err(|error| format!("HARNESS_BOOTSTRAP_INVALID: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requires_https_except_for_loopback_development() {
        assert_eq!(
            validate_chat_url("https://chat.mundusx.ai/").unwrap(),
            "https://chat.mundusx.ai"
        );
        assert!(validate_chat_url("http://127.0.0.1:8787").is_ok());
        assert!(validate_chat_url("http://chat.mundusx.ai").is_err());
    }

    #[test]
    fn approval_must_stay_on_the_configured_chat_origin() {
        assert!(validate_approval_url(
            "https://chat.mundusx.ai",
            "https://chat.mundusx.ai/runner/connect#token=secret"
        )
        .is_ok());
        assert!(validate_approval_url(
            "https://chat.mundusx.ai",
            "https://attacker.example/runner/connect#token=secret"
        )
        .is_err());
    }
}
