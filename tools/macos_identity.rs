use serde::Deserialize;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Debug, Deserialize)]
struct HelperIdentity {
    public_key_hex: String,
    fingerprint: String,
    created: bool,
}

pub struct SecureIdentity {
    pub public_key_hex: String,
    pub fingerprint: String,
    pub created: bool,
}

const HELPER_SOURCE: &str = include_str!("macos_identity_helper.swift");

pub fn ensure_identity(storage_dir: &Path) -> io::Result<SecureIdentity> {
    let output = run_helper(storage_dir, "ensure", &[])?;
    let parsed: HelperIdentity = serde_json::from_str(&output)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    Ok(SecureIdentity {
        public_key_hex: parsed.public_key_hex,
        fingerprint: parsed.fingerprint,
        created: parsed.created,
    })
}

pub fn sign_message(storage_dir: &Path, message: &[u8]) -> io::Result<String> {
    let mut command = helper_command(storage_dir)?;
    command.arg("sign");
    command.stdin(Stdio::piped());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let mut child = command.spawn()?;
    if let Some(stdin) = child.stdin.as_mut() {
        stdin.write_all(message)?;
    }

    let output = child.wait_with_output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(io::Error::new(io::ErrorKind::Other, stderr.trim().to_string()));
    }

    let signature = String::from_utf8(output.stdout)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    Ok(signature.trim().to_string())
}

fn run_helper(storage_dir: &Path, command_name: &str, args: &[&str]) -> io::Result<String> {
    let output = helper_command(storage_dir)?
        .arg(command_name)
        .args(args)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(io::Error::new(io::ErrorKind::Other, stderr.trim().to_string()));
    }

    String::from_utf8(output.stdout)
        .map(|output| output.trim().to_string())
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn helper_command(storage_dir: &Path) -> io::Result<Command> {
    let binary = ensure_helper_binary(storage_dir)?;
    Ok(Command::new(binary))
}

fn ensure_helper_binary(storage_dir: &Path) -> io::Result<PathBuf> {
    let bin_dir = storage_dir.join("bin");
    let binary_path = bin_dir.join("opengpu-device-identity-helper");
    let source_path = bin_dir.join("opengpu-device-identity-helper.swift");

    if binary_path.exists() {
        return Ok(binary_path);
    }

    fs::create_dir_all(&bin_dir)?;
    fs::write(&source_path, HELPER_SOURCE)?;

    let source_arg = source_path.to_string_lossy().into_owned();
    let binary_arg = binary_path.to_string_lossy().into_owned();
    let output = Command::new("swiftc")
        .args(["-O", &source_arg, "-o", &binary_arg])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(io::Error::new(
            io::ErrorKind::Other,
            format!("failed to compile macOS identity helper: {}", stderr.trim()),
        ));
    }

    Ok(binary_path)
}
