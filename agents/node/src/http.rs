use crate::identity::DeviceIdentity;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct HttpEndpoint {
    pub host: String,
    pub port: u16,
    pub path: String,
}

pub fn parse_http_endpoint(input: &str, default_path: &str) -> Result<HttpEndpoint, String> {
    let trimmed = input.trim();
    if trimmed.starts_with("https://") {
        return Err(
            "prototype control plane client only supports http://; set control-plane-url to http://127.0.0.1:8787"
                .to_string(),
        );
    }

    let without_scheme = trimmed.strip_prefix("http://").ok_or_else(|| {
        "control plane URL must use http://; set control-plane-url to http://127.0.0.1:8787"
            .to_string()
    })?;

    let mut parts = without_scheme.splitn(2, '/');
    let host_port = parts.next().unwrap_or_default();
    let path = parts
        .next()
        .map(|tail| format!("/{}", tail.trim_start_matches('/')))
        .unwrap_or_else(|| default_path.to_string());

    let mut host_parts = host_port.splitn(2, ':');
    let host = host_parts.next().unwrap_or_default().to_string();
    if host.is_empty() {
        return Err("control plane host cannot be empty".to_string());
    }

    let port = host_parts
        .next()
        .map(|value| {
            value
                .parse::<u16>()
                .map_err(|_| "invalid control plane port".to_string())
        })
        .transpose()?
        .unwrap_or(80);

    Ok(HttpEndpoint { host, port, path })
}

pub fn post_json<T: Serialize>(
    control_plane_url: &str,
    path: &str,
    payload: &T,
) -> Result<String, String> {
    let endpoint = parse_http_endpoint(control_plane_url, path)?;
    let body = serde_json::to_string(payload).map_err(|error| error.to_string())?;
    let request = format!(
        "POST {} HTTP/1.1\r\nHost: {}:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        endpoint.path,
        endpoint.host,
        endpoint.port,
        body.len(),
        body
    );
    send_request(&endpoint.host, endpoint.port, &request)
}

pub fn get_json<T: DeserializeOwned>(control_plane_url: &str, path: &str) -> Result<T, String> {
    let endpoint = parse_http_endpoint(control_plane_url, path)?;
    let request = format!(
        "GET {} HTTP/1.1\r\nHost: {}:{}\r\nConnection: close\r\n\r\n",
        endpoint.path, endpoint.host, endpoint.port
    );
    let response = send_request(&endpoint.host, endpoint.port, &request)?;
    parse_json_body(&response)
}

pub fn post_json_body<T: Serialize, R: DeserializeOwned>(
    control_plane_url: &str,
    path: &str,
    payload: &T,
) -> Result<R, String> {
    let endpoint = parse_http_endpoint(control_plane_url, path)?;
    let body = serde_json::to_string(payload).map_err(|error| error.to_string())?;
    let request = format!(
        "POST {} HTTP/1.1\r\nHost: {}:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        endpoint.path,
        endpoint.host,
        endpoint.port,
        body.len(),
        body
    );
    let response = send_request(&endpoint.host, endpoint.port, &request)?;
    parse_json_body(&response)
}

pub fn signed_post_json<T: Serialize>(
    control_plane_url: &str,
    path: &str,
    node_id: &str,
    identity: &DeviceIdentity,
    payload: &T,
) -> Result<String, String> {
    signed_request(control_plane_url, "POST", path, node_id, identity, payload)
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
    let response = signed_request(control_plane_url, "POST", path, node_id, identity, payload)?;
    parse_json_body(&response)
}

fn signed_request<T: Serialize>(
    control_plane_url: &str,
    method: &str,
    path: &str,
    node_id: &str,
    identity: &DeviceIdentity,
    payload: &T,
) -> Result<String, String> {
    let endpoint = parse_http_endpoint(control_plane_url, path)?;
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
    let request = if method == "GET" {
        format!(
            "GET {} HTTP/1.1\r\nHost: {}:{}\r\nX-MundusX-Node-Id: {}\r\nX-MundusX-Public-Key: {}\r\nX-MundusX-Timestamp: {}\r\nX-MundusX-Signature: {}\r\nConnection: close\r\n\r\n",
            endpoint.path,
            endpoint.host,
            endpoint.port,
            node_id,
            identity.public_key_hex,
            timestamp,
            signature
        )
    } else {
        format!(
            "POST {} HTTP/1.1\r\nHost: {}:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nX-MundusX-Node-Id: {}\r\nX-MundusX-Public-Key: {}\r\nX-MundusX-Timestamp: {}\r\nX-MundusX-Signature: {}\r\nConnection: close\r\n\r\n{}",
            endpoint.path,
            endpoint.host,
            endpoint.port,
            body.len(),
            node_id,
            identity.public_key_hex,
            timestamp,
            signature,
            body
        )
    };
    send_request(&endpoint.host, endpoint.port, &request)
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
        .unwrap_or_default();
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

fn parse_response_status(response: &str) -> Result<(), String> {
    let Some(status_line) = response.lines().next() else {
        return Err("empty HTTP response".to_string());
    };

    let mut parts = status_line.split_whitespace();
    let _http_version = parts
        .next()
        .ok_or_else(|| "malformed HTTP status line".to_string())?;
    let status_code = parts
        .next()
        .ok_or_else(|| "malformed HTTP status line".to_string())?;
    let status_code_num = status_code
        .parse::<u16>()
        .map_err(|_| format!("invalid HTTP status code `{status_code}`"))?;

    if (200..300).contains(&status_code_num) {
        return Ok(());
    }

    let body = response
        .split_once("\r\n\r\n")
        .map(|(_, body)| body.trim())
        .filter(|body| !body.is_empty());
    match body {
        Some(body) => Err(format!("HTTP {status_code_num}: {body}")),
        None => Err(format!("HTTP {status_code_num}")),
    }
}

fn send_request(host: &str, port: u16, request: &str) -> Result<String, String> {
    let timeout = request_timeout();
    let address = (host, port)
        .to_socket_addrs()
        .map_err(|error| format!("resolve failed: {error}"))?
        .next()
        .ok_or_else(|| "resolve failed: no address found".to_string())?;
    let mut stream = TcpStream::connect_timeout(&address, timeout)
        .map_err(|error| format!("connect failed: {error}"))?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|error| format!("read timeout setup failed: {error}"))?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|error| format!("write timeout setup failed: {error}"))?;
    stream
        .write_all(request.as_bytes())
        .map_err(|error| format!("write failed: {error}"))?;
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .map_err(|error| format!("read failed: {error}"))?;
    parse_response_status(&response)?;
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::{parse_response_status, request_timeout};
    use std::env;
    use std::sync::{Mutex, OnceLock};
    use std::time::Duration;

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn parse_response_status_rejects_non_success_status_codes() {
        let response =
            "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 16\r\n\r\n{\"error\":\"boom\"}";
        let error = parse_response_status(response).expect_err("non-2xx response should fail");

        assert!(error.contains("500"), "unexpected error: {error}");
    }

    #[test]
    fn parse_response_status_accepts_success_status_codes() {
        let response = "HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n";

        parse_response_status(response).expect("2xx response should succeed");
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
