use crate::identity::DeviceIdentity;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct ControlPlaneEndpoint {
    pub url: String,
    pub path: String,
}

pub fn control_plane_endpoint(
    control_plane_url: &str,
    path: &str,
) -> Result<ControlPlaneEndpoint, String> {
    let base = control_plane_url.trim().trim_end_matches('/');
    if !(base.starts_with("http://") || base.starts_with("https://")) {
        return Err("control-plane-url must start with http:// or https://".to_string());
    }

    if path.is_empty() || !path.starts_with('/') {
        return Err("control-plane API path must start with /".to_string());
    }

    Ok(ControlPlaneEndpoint {
        url: format!("{base}{path}"),
        path: path.to_string(),
    })
}

pub fn post_json<T: Serialize>(
    control_plane_url: &str,
    path: &str,
    payload: &T,
) -> Result<String, String> {
    let endpoint = control_plane_endpoint(control_plane_url, path)?;
    let body = serde_json::to_string(payload).map_err(|error| error.to_string())?;
    send_request("POST", &endpoint.url, Vec::new(), Some(body))
}

pub fn get_json<T: DeserializeOwned>(control_plane_url: &str, path: &str) -> Result<T, String> {
    let endpoint = control_plane_endpoint(control_plane_url, path)?;
    let response = send_request("GET", &endpoint.url, Vec::new(), None)?;
    parse_json_body(&response)
}

pub fn post_json_body<T: Serialize, R: DeserializeOwned>(
    control_plane_url: &str,
    path: &str,
    payload: &T,
) -> Result<R, String> {
    let endpoint = control_plane_endpoint(control_plane_url, path)?;
    let body = serde_json::to_string(payload).map_err(|error| error.to_string())?;
    let response = send_request("POST", &endpoint.url, Vec::new(), Some(body))?;
    parse_json_body(&response)
}

pub fn signed_post_json<T: Serialize>(
    control_plane_url: &str,
    path: &str,
    node_id: &str,
    identity: &DeviceIdentity,
    payload: &T,
) -> Result<String, String> {
    signed_request(
        control_plane_url,
        "POST",
        path,
        "X-MundusX-Node-Id",
        node_id,
        identity,
        payload,
    )
}

pub fn signed_get_json<T: DeserializeOwned>(
    control_plane_url: &str,
    path: &str,
    node_id: &str,
    identity: &DeviceIdentity,
) -> Result<T, String> {
    let response = signed_request(
        control_plane_url,
        "GET",
        path,
        "X-MundusX-Node-Id",
        node_id,
        identity,
        &serde_json::json!({}),
    )?;
    parse_json_body(&response)
}

pub fn signed_post_json_body<T: Serialize, R: DeserializeOwned>(
    control_plane_url: &str,
    path: &str,
    node_id: &str,
    identity: &DeviceIdentity,
    payload: &T,
) -> Result<R, String> {
    let response = signed_request(
        control_plane_url,
        "POST",
        path,
        "X-MundusX-Node-Id",
        node_id,
        identity,
        payload,
    )?;
    parse_json_body(&response)
}

pub fn signed_post_json_body_with_agent<T: Serialize, R: DeserializeOwned>(
    agent: &ureq::Agent,
    control_plane_url: &str,
    path: &str,
    node_id: &str,
    identity: &DeviceIdentity,
    payload: &T,
) -> Result<R, String> {
    let response = signed_request_with_agent(
        agent,
        control_plane_url,
        "POST",
        path,
        "X-MundusX-Node-Id",
        node_id,
        identity,
        payload,
    )?;
    parse_json_body(&response)
}

pub fn signed_runner_get_json<T: DeserializeOwned>(
    control_plane_url: &str,
    path: &str,
    runner_id: &str,
    identity: &DeviceIdentity,
) -> Result<T, String> {
    let response = signed_request(
        control_plane_url,
        "GET",
        path,
        "X-MundusX-Runner-Id",
        runner_id,
        identity,
        &serde_json::json!({}),
    )?;
    parse_json_body(&response)
}

pub fn signed_runner_post_json_body<T: Serialize, R: DeserializeOwned>(
    control_plane_url: &str,
    path: &str,
    runner_id: &str,
    identity: &DeviceIdentity,
    payload: &T,
) -> Result<R, String> {
    let response = signed_request(
        control_plane_url,
        "POST",
        path,
        "X-MundusX-Runner-Id",
        runner_id,
        identity,
        payload,
    )?;
    parse_json_body(&response)
}

fn signed_request<T: Serialize>(
    control_plane_url: &str,
    method: &str,
    path: &str,
    identity_header: &str,
    identity_id: &str,
    identity: &DeviceIdentity,
    payload: &T,
) -> Result<String, String> {
    let agent = request_agent();
    signed_request_with_agent(
        &agent,
        control_plane_url,
        method,
        path,
        identity_header,
        identity_id,
        identity,
        payload,
    )
}

fn signed_request_with_agent<T: Serialize>(
    agent: &ureq::Agent,
    control_plane_url: &str,
    method: &str,
    path: &str,
    identity_header: &str,
    identity_id: &str,
    identity: &DeviceIdentity,
    payload: &T,
) -> Result<String, String> {
    let endpoint = control_plane_endpoint(control_plane_url, path)?;
    let body = if method == "GET" {
        String::new()
    } else {
        serde_json::to_string(payload).map_err(|error| error.to_string())?
    };
    let timestamp = unix_seconds_string();
    let message = format!("{method}\n{}\n{timestamp}\n{body}", endpoint.path);
    let signature = identity
        .sign_hex(&message)
        .map_err(|error| error.to_string())?;
    let headers = vec![
        (identity_header, identity_id.to_string()),
        ("X-MundusX-Public-Key", identity.public_key_hex.clone()),
        ("X-MundusX-Timestamp", timestamp),
        ("X-MundusX-Signature", signature),
    ];
    let payload = if method == "GET" { None } else { Some(body) };
    send_request_with_agent(agent, method, &endpoint.url, headers, payload)
}

fn unix_seconds_string() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs().to_string())
        .unwrap_or_else(|_| "0".to_string())
}

fn parse_json_body<T: DeserializeOwned>(response: &str) -> Result<T, String> {
    let body = response
        .split_once("\r\n\r\n")
        .map(|(_, body)| body)
        .unwrap_or(response);
    serde_json::from_str(body).map_err(|error| error.to_string())
}

fn request_timeout() -> Duration {
    std::env::var("OPENGPU_HTTP_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .map(Duration::from_millis)
        .unwrap_or_else(|| Duration::from_secs(5))
}

pub fn request_agent() -> ureq::Agent {
    ureq::AgentBuilder::new().timeout(request_timeout()).build()
}

fn control_plane_error(error: ureq::Error) -> String {
    match error {
        ureq::Error::Status(code, response) => {
            let body = response
                .into_string()
                .unwrap_or_default()
                .trim()
                .to_string();
            if body.is_empty() {
                format!("HTTP {code}")
            } else {
                format!("HTTP {code}: {body}")
            }
        }
        ureq::Error::Transport(error) => format!("transport failed: {error}"),
    }
}

fn send_request(
    method: &str,
    url: &str,
    headers: Vec<(&str, String)>,
    body: Option<String>,
) -> Result<String, String> {
    let agent = request_agent();
    send_request_with_agent(&agent, method, url, headers, body)
}

fn send_request_with_agent(
    agent: &ureq::Agent,
    method: &str,
    url: &str,
    headers: Vec<(&str, String)>,
    body: Option<String>,
) -> Result<String, String> {
    let mut request = match method {
        "GET" => agent.get(url),
        "POST" => agent.post(url),
        other => return Err(format!("unsupported HTTP method `{other}`")),
    };
    for (name, value) in headers {
        request = request.set(name, &value);
    }
    if body.is_some() {
        request = request.set("Content-Type", "application/json");
    }

    let response = match body {
        Some(body) => request.send_string(&body).map_err(control_plane_error)?,
        None => request.call().map_err(control_plane_error)?,
    };
    let status = response.status();
    let reason = response.status_text().to_string();
    let response_body = response.into_string().map_err(|error| error.to_string())?;
    Ok(format!("HTTP/1.1 {status} {reason}\r\n\r\n{response_body}"))
}

#[cfg(test)]
mod tests {
    use super::{control_plane_endpoint, request_timeout};
    use std::env;
    use std::sync::{Mutex, OnceLock};
    use std::time::Duration;

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn control_plane_endpoint_accepts_https_and_local_http() {
        let hosted = control_plane_endpoint("https://uat.mundusx.ai", "/v1/register")
            .expect("hosted endpoint");
        assert_eq!(hosted.url, "https://uat.mundusx.ai/v1/register");
        assert_eq!(hosted.path, "/v1/register");

        let local = control_plane_endpoint("http://127.0.0.1:8787/", "/v1/heartbeat")
            .expect("local endpoint");
        assert_eq!(local.url, "http://127.0.0.1:8787/v1/heartbeat");
        assert_eq!(local.path, "/v1/heartbeat");
    }

    #[test]
    fn control_plane_endpoint_rejects_invalid_urls_and_paths() {
        assert!(control_plane_endpoint("uat.mundusx.ai", "/v1/register").is_err());
        assert!(control_plane_endpoint("ftp://uat.mundusx.ai", "/v1/register").is_err());
        assert!(control_plane_endpoint("https://uat.mundusx.ai", "v1/register").is_err());
    }

    #[test]
    fn request_timeout_uses_env_override() {
        let _guard = env_lock().lock().expect("env lock");
        env::set_var("OPENGPU_HTTP_TIMEOUT_MS", "25");
        let timeout = request_timeout();
        env::remove_var("OPENGPU_HTTP_TIMEOUT_MS");

        assert_eq!(timeout, Duration::from_millis(25));
    }

    #[test]
    fn request_timeout_ignores_invalid_override() {
        let _guard = env_lock().lock().expect("env lock");
        env::set_var("OPENGPU_HTTP_TIMEOUT_MS", "bad");
        let timeout = request_timeout();
        env::remove_var("OPENGPU_HTTP_TIMEOUT_MS");

        assert_eq!(timeout, Duration::from_secs(5));
    }
}
