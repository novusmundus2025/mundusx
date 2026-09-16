//! Endpoint-scoped credentials shared by the CLI and background node.
use serde::{Deserialize, Serialize};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use url::Url;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum TokenHeader {
    Bearer,
    CoderSessionToken,
}

// Deliberately does not implement Debug: credentials must not appear in diagnostics.
#[derive(Serialize, Deserialize)]
struct Credential {
    base_url: String,
    header: TokenHeader,
    token: String,
}

pub fn config_dir() -> PathBuf {
    std::env::var_os("OPENGPU_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|dir| dir.join(".opengpu")))
        .unwrap_or_else(|| PathBuf::from(".opengpu"))
}

fn credential_path(dir: &Path) -> PathBuf {
    dir.join(if cfg!(windows) {
        "control-plane-auth.dpapi"
    } else {
        "control-plane-auth.json"
    })
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn parse_base(value: &str) -> io::Result<Url> {
    let url = Url::parse(value.trim()).map_err(|_| invalid("invalid control-plane URL"))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid(
            "control-plane URL must be an HTTP(S) base URL without credentials, query, or fragment",
        ));
    }
    Ok(url)
}

impl Credential {
    fn matches(&self, target: &str) -> bool {
        let (Ok(base), Ok(target)) = (parse_base(&self.base_url), Url::parse(target)) else {
            return false;
        };
        let path = base.path().trim_end_matches('/');
        base.origin() == target.origin()
            && target.username().is_empty()
            && target.password().is_none()
            && (target.path() == path || target.path().starts_with(&format!("{path}/")))
    }

    fn apply(&self, request: ureq::Request) -> ureq::Request {
        if !self.matches(request.url()) {
            return request;
        }
        match self.header {
            TokenHeader::Bearer => request.set("Authorization", &format!("Bearer {}", self.token)),
            TokenHeader::CoderSessionToken => request.set("Coder-Session-Token", &self.token),
        }
    }
}

pub fn store(base_url: &str, token: &str, header: TokenHeader) -> io::Result<PathBuf> {
    store_in(&config_dir(), base_url, token, header)
}

fn store_in(dir: &Path, base_url: &str, token: &str, header: TokenHeader) -> io::Result<PathBuf> {
    let base_url = parse_base(base_url)?.to_string();
    let token = token.trim();
    if token.is_empty() || !token.bytes().all(|byte| (0x21..=0x7e).contains(&byte)) {
        return Err(invalid(
            "token must contain only non-space printable ASCII characters",
        ));
    }
    let credential = Credential {
        base_url,
        token: token.to_string(),
        header,
    };
    let bytes =
        serde_json::to_vec(&credential).map_err(|_| invalid("could not encode credential"))?;
    let bytes = protect(&bytes)?;
    std::fs::create_dir_all(dir)?;
    let path = credential_path(dir);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(&bytes)?;
    Ok(path)
}

fn load_from(dir: &Path) -> io::Result<Option<Credential>> {
    let bytes = match std::fs::read(credential_path(dir)) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let credential: Credential = serde_json::from_slice(&unprotect(&bytes)?)
        .map_err(|_| invalid("invalid saved control-plane credential; log in again"))?;
    parse_base(&credential.base_url)?;
    if credential.token.is_empty()
        || !credential
            .token
            .bytes()
            .all(|byte| (0x21..=0x7e).contains(&byte))
    {
        return Err(invalid("invalid saved control-plane token; log in again"));
    }
    Ok(Some(credential))
}

pub fn present_for(base_url: &str) -> bool {
    load_from(&config_dir())
        .ok()
        .flatten()
        .is_some_and(|credential| credential.matches(base_url))
}

pub fn clear() -> io::Result<()> {
    match std::fs::remove_file(credential_path(&config_dir())) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

/// Call only for control-plane traffic. Callers disable redirects on their agents
/// so a gateway cannot forward the custom Coder header to a different endpoint.
pub fn apply(request: ureq::Request) -> Result<ureq::Request, String> {
    let credential = load_from(&config_dir()).map_err(|_| {
        "could not read control-plane credential; run opengpu login again".to_string()
    })?;
    Ok(match credential {
        Some(credential) => credential.apply(request),
        None => request,
    })
}

#[cfg(not(windows))]
fn protect(bytes: &[u8]) -> io::Result<Vec<u8>> {
    Ok(bytes.to_vec())
}
#[cfg(not(windows))]
fn unprotect(bytes: &[u8]) -> io::Result<Vec<u8>> {
    Ok(bytes.to_vec())
}

#[cfg(windows)]
fn protect(bytes: &[u8]) -> io::Result<Vec<u8>> {
    crypt(bytes, true)
}
#[cfg(windows)]
fn unprotect(bytes: &[u8]) -> io::Result<Vec<u8>> {
    crypt(bytes, false)
}

#[cfg(windows)]
fn crypt(bytes: &[u8], encrypt: bool) -> io::Result<Vec<u8>> {
    use windows_sys::Win32::Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };
    let input = CRYPT_INTEGER_BLOB {
        cbData: bytes.len() as u32,
        pbData: bytes.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    let ok = unsafe {
        if encrypt {
            CryptProtectData(
                &input,
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        }
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    let result =
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
    unsafe {
        windows_sys::Win32::Foundation::LocalFree(output.pbData as _);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::net::TcpListener;
    use std::time::Duration;

    fn credential(base_url: &str, header: TokenHeader) -> Credential {
        Credential {
            base_url: base_url.into(),
            header,
            token: "test-only-token".into(),
        }
    }

    #[test]
    fn token_is_scoped_to_origin_and_base_path() {
        let credential = credential(
            "https://private.example/control",
            TokenHeader::CoderSessionToken,
        );
        for target in [
            "https://private.example/control",
            "https://private.example/control/v1/jobs",
        ] {
            assert!(credential
                .apply(ureq::get(target))
                .header("Coder-Session-Token")
                .is_some());
        }
        for target in [
            "https://other.example/control/v1/jobs",
            "http://private.example/control/v1/jobs",
            "https://private.example:8443/control/v1/jobs",
            "https://private.example/control-other",
            "https://private.example/health",
            "https://private.example/control/../other",
            "https://user@private.example/control/v1/jobs",
        ] {
            assert!(credential
                .apply(ureq::get(target))
                .header("Coder-Session-Token")
                .is_none());
        }
    }

    #[test]
    fn coder_and_bearer_requests_reach_server_with_expected_header_and_signatures() {
        for (mode, expected) in [
            (
                TokenHeader::CoderSessionToken,
                "coder-session-token: test-only-token",
            ),
            (TokenHeader::Bearer, "authorization: Bearer test-only-token"),
        ] {
            for method in ["GET", "POST"] {
                let listener = TcpListener::bind("127.0.0.1:0").unwrap();
                let base = format!("http://{}", listener.local_addr().unwrap());
                let server = std::thread::spawn(move || {
                    let (mut stream, _) = listener.accept().unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(5)))
                        .unwrap();
                    let mut raw = Vec::new();
                    let mut byte = [0];
                    while !raw.ends_with(b"\r\n\r\n") {
                        stream.read_exact(&mut byte).unwrap();
                        raw.push(byte[0]);
                    }
                    stream
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
                        )
                        .unwrap();
                    String::from_utf8(raw).unwrap()
                });
                let agent = ureq::AgentBuilder::new()
                    .redirects(0)
                    .timeout(Duration::from_secs(5))
                    .build();
                let request = agent
                    .request(method, &format!("{base}/v1/heartbeat"))
                    .set("X-MundusX-Signature", "signed-payload");
                let request = credential(&base, mode).apply(request);
                if method == "POST" {
                    request.send_string("{}").unwrap();
                } else {
                    request.call().unwrap();
                }
                let raw = server.join().unwrap().to_ascii_lowercase();
                assert!(raw.contains(&expected.to_ascii_lowercase()));
                assert!(raw.contains("x-mundusx-signature: signed-payload"));
                if mode == TokenHeader::CoderSessionToken {
                    assert!(!raw.contains("authorization:"));
                } else {
                    assert!(!raw.contains("coder-session-token:"));
                }
            }
        }
    }

    #[test]
    fn stored_credential_round_trips_without_plaintext_on_windows() {
        let dir = std::env::temp_dir().join(format!(
            "mundusx-auth-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        assert!(load_from(&dir).unwrap().is_none());
        let path = store_in(
            &dir,
            "https://private.example",
            "test-only-token",
            TokenHeader::CoderSessionToken,
        )
        .unwrap();
        let saved = load_from(&dir).unwrap().unwrap();
        assert_eq!(saved.header, TokenHeader::CoderSessionToken);
        assert_eq!(saved.token, "test-only-token");
        assert!(saved.matches("https://private.example/v1/register"));
        #[cfg(windows)]
        assert!(!std::fs::read(&path)
            .unwrap()
            .windows(15)
            .any(|bytes| bytes == b"test-only-token"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        assert!(store_in(
            &dir,
            "https://private.example",
            "bad\r\nheader",
            TokenHeader::Bearer
        )
        .is_err());
        assert!(store_in(&dir, "https://private.example", "", TokenHeader::Bearer).is_err());
        std::fs::write(&path, b"invalid saved credential").unwrap();
        assert!(load_from(&dir).is_err());
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn rejects_urls_with_embedded_credentials_or_query_tokens() {
        for url in [
            "[https://private.example](https://private.example)",
            "https://user:password@private.example",
            "https://private.example?token=secret",
            "https://private.example#token",
            "ftp://private.example",
        ] {
            assert!(parse_base(url).is_err());
        }
    }
}
