use serde::de::DeserializeOwned;
use serde::Serialize;
use std::io::{Read, Write};
use std::net::TcpStream;

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

    let without_scheme = trimmed
        .strip_prefix("http://")
        .ok_or_else(|| {
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
        .map(|value| value.parse::<u16>().map_err(|_| "invalid control plane port".to_string()))
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

fn parse_json_body<T: DeserializeOwned>(response: &str) -> Result<T, String> {
    let body = response
        .split_once("\r\n\r\n")
        .map(|(_, body)| body)
        .unwrap_or_default();
    serde_json::from_str(body).map_err(|error| error.to_string())
}

fn send_request(host: &str, port: u16, request: &str) -> Result<String, String> {
    let mut stream =
        TcpStream::connect((host, port)).map_err(|error| format!("connect failed: {error}"))?;
    stream
        .write_all(request.as_bytes())
        .map_err(|error| format!("write failed: {error}"))?;
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .map_err(|error| format!("read failed: {error}"))?;
    Ok(response)
}
