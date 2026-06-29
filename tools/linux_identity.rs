#![allow(dead_code)]

use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const IDENTITY_SECRET_SERVICE: &str = "com.opengpu.device.identity.secret";
const TRUST_PATH_SECRET_SERVICE: &str = "secret-service://mundusx/device-identity";
const TRUST_PATH_LOCAL_FALLBACK: &str = "local-encrypted-fallback://mundusx/device-identity";

#[derive(Debug, Deserialize)]
struct StoredIdentity {
    public_key_hex: String,
    fingerprint: String,
    #[serde(default)]
    private_key_hex: String,
    #[serde(default)]
    encrypted_private_key_hex: String,
    #[serde(default)]
    nonce_hex: String,
    #[serde(default)]
    keychain_label_hex: Option<String>,
}

#[derive(Debug, Serialize)]
struct PersistedIdentity {
    public_key_hex: String,
    fingerprint: String,
    encrypted_private_key_hex: String,
    nonce_hex: String,
    keychain_label_hex: String,
}

pub struct SecureIdentity {
    pub public_key_hex: String,
    pub fingerprint: String,
    pub keychain_label_hex: String,
    pub encrypted_private_key_hex: String,
    pub nonce_hex: String,
    pub created: bool,
}

pub fn ensure_identity(storage_dir: &Path) -> io::Result<SecureIdentity> {
    if let Some(existing) = load_stored_identity(storage_dir)? {
        reject_plaintext_identity(&existing)?;
        if !existing.encrypted_private_key_hex.is_empty() && !existing.nonce_hex.is_empty() {
            return Ok(SecureIdentity {
                public_key_hex: existing.public_key_hex,
                fingerprint: existing.fingerprint,
                keychain_label_hex: existing.keychain_label_hex.unwrap_or_else(|| {
                    machine_label_hex().unwrap_or_else(|_| "unknown".to_string())
                }),
                encrypted_private_key_hex: existing.encrypted_private_key_hex,
                nonce_hex: existing.nonce_hex,
                created: false,
            });
        }
    }

    let mut private_key = [0u8; 32];
    OsRng.fill_bytes(&mut private_key);
    let signing_key = SigningKey::from_bytes(&private_key);
    let verifying_key = signing_key.verifying_key();
    let public_key_hex = hex::encode(verifying_key.to_bytes());
    let fingerprint = fingerprint_from_public_key(verifying_key.as_bytes());

    let mut nonce = [0u8; 12];
    OsRng.fill_bytes(&mut nonce);
    let secret = machine_secret_key()?;
    let encrypted = xor_crypt(&private_key, &secret, &nonce);
    private_key.fill(0);
    let label_hex =
        machine_label_hex().unwrap_or_else(|_| hex::encode(IDENTITY_SECRET_SERVICE.as_bytes()));

    let persisted = PersistedIdentity {
        public_key_hex: public_key_hex.clone(),
        fingerprint: fingerprint.clone(),
        encrypted_private_key_hex: hex::encode(encrypted),
        nonce_hex: hex::encode(nonce),
        keychain_label_hex: label_hex.clone(),
    };
    save_stored_identity(storage_dir, &persisted)?;

    Ok(SecureIdentity {
        public_key_hex,
        fingerprint,
        keychain_label_hex: label_hex,
        encrypted_private_key_hex: persisted.encrypted_private_key_hex,
        nonce_hex: persisted.nonce_hex,
        created: true,
    })
}

pub fn sign_message(storage_dir: &Path, message: &[u8]) -> io::Result<String> {
    let stored = load_stored_identity(storage_dir)?.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "missing Linux device identity; run `opengpu start` first",
        )
    })?;

    reject_plaintext_identity(&stored)?;
    if stored.encrypted_private_key_hex.trim().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Linux device identity is missing protected key material",
        ));
    }

    let secret = machine_secret_key()?;
    let nonce = hex::decode(&stored.nonce_hex).map_err(invalid_identity)?;
    if nonce.len() != 12 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "nonce must be 12 bytes",
        ));
    }

    let encrypted_private =
        hex::decode(&stored.encrypted_private_key_hex).map_err(invalid_identity)?;
    let private_key_bytes_vec = xor_crypt(&encrypted_private, &secret, nonce.as_slice());
    let private_bytes: [u8; 32] = private_key_bytes_vec
        .clone()
        .try_into()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "private key must be 32 bytes"))?;
    let signing_key = SigningKey::from_bytes(&private_bytes);
    let signature = signing_key.sign(message);
    let mut private_key_bytes = private_key_bytes_vec;
    private_key_bytes.fill(0);
    Ok(hex::encode(signature.to_bytes()))
}

pub fn verify_message(
    _storage_dir: &Path,
    public_key_hex: &str,
    message: &[u8],
    signature_hex: &str,
) -> io::Result<bool> {
    let public_bytes = hex::decode(public_key_hex).map_err(invalid_identity)?;
    let public_bytes: [u8; 32] = public_bytes
        .try_into()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "public key must be 32 bytes"))?;
    let verifying_key = VerifyingKey::from_bytes(&public_bytes).map_err(invalid_identity)?;

    let signature_bytes = hex::decode(signature_hex).map_err(invalid_identity)?;
    let signature =
        ed25519_dalek::Signature::from_slice(&signature_bytes).map_err(invalid_identity)?;
    Ok(verifying_key.verify(message, &signature).is_ok())
}

pub fn trust_path() -> &'static str {
    if load_machine_secret_from_secret_service()
        .ok()
        .flatten()
        .is_some()
    {
        TRUST_PATH_SECRET_SERVICE
    } else {
        TRUST_PATH_LOCAL_FALLBACK
    }
}

fn reject_plaintext_identity(stored: &StoredIdentity) -> io::Result<()> {
    if stored.private_key_hex.trim().is_empty() {
        return Ok(());
    }

    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        "plaintext Linux device identity is no longer accepted; remove identity.json and re-enroll",
    ))
}

fn load_stored_identity(storage_dir: &Path) -> io::Result<Option<StoredIdentity>> {
    let path = identity_path(storage_dir);
    if !path.exists() {
        return Ok(None);
    }

    let raw = fs::read_to_string(path)?;
    let stored: StoredIdentity = serde_json::from_str(&raw)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    Ok(Some(stored))
}

fn save_stored_identity(storage_dir: &Path, identity: &PersistedIdentity) -> io::Result<()> {
    let path = identity_path(storage_dir);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let data = serde_json::to_string_pretty(identity).expect("identity serialization");
    fs::write(path, format!("{data}\n"))?;
    Ok(())
}

fn identity_path(storage_dir: &Path) -> PathBuf {
    storage_dir.join("identity.json")
}

fn machine_secret_key() -> io::Result<[u8; 32]> {
    if let Some(secret) = load_machine_secret_from_secret_service()? {
        return Ok(secret);
    }

    let mut key = [0u8; 32];
    OsRng.fill_bytes(&mut key);
    if store_machine_secret_in_secret_service(&key).is_ok() {
        return Ok(key);
    }

    fallback_machine_secret_key()
}

fn secret_service_account() -> String {
    machine_label_hex().unwrap_or_else(|_| "unknown-machine".to_string())
}

fn load_machine_secret_from_secret_service() -> io::Result<Option<[u8; 32]>> {
    let account = secret_service_account();
    let output = Command::new("secret-tool")
        .args([
            "lookup",
            "service",
            IDENTITY_SECRET_SERVICE,
            "account",
            &account,
        ])
        .stderr(Stdio::null())
        .output();

    let Ok(output) = output else {
        return Ok(None);
    };

    if !output.status.success() {
        return Ok(None);
    }

    let secret_hex = String::from_utf8(output.stdout)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let secret = hex::decode(secret_hex.trim()).map_err(invalid_identity)?;
    let secret: [u8; 32] = secret.try_into().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "machine secret must be 32 bytes",
        )
    })?;
    Ok(Some(secret))
}

fn store_machine_secret_in_secret_service(secret: &[u8; 32]) -> io::Result<()> {
    let account = secret_service_account();
    let secret_hex = hex::encode(secret);
    let mut child = Command::new("secret-tool")
        .args([
            "store",
            "--label",
            "MundusX device identity secret",
            "service",
            IDENTITY_SECRET_SERVICE,
            "account",
            &account,
        ])
        .stdin(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;

    if let Some(stdin) = child.stdin.as_mut() {
        use std::io::Write;
        stdin.write_all(secret_hex.as_bytes())?;
    }

    let status = child.wait()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "unable to store machine secret in Secret Service",
        ))
    }
}

fn machine_label_hex() -> io::Result<String> {
    let machine_id = machine_identifier().unwrap_or_else(|_| "unknown-machine".to_string());
    Ok(hex::encode(machine_id.as_bytes()))
}

fn fallback_machine_secret_key() -> io::Result<[u8; 32]> {
    let machine_id = machine_identifier().unwrap_or_else(|_| "unknown-machine".to_string());
    Ok(fallback_machine_secret_key_from_machine_id(&machine_id))
}

fn fallback_machine_secret_key_from_machine_id(machine_id: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(IDENTITY_SECRET_SERVICE.as_bytes());
    hasher.update(machine_id.as_bytes());
    let digest = hasher.finalize();
    let mut key = [0u8; 32];
    key.copy_from_slice(&digest);
    key
}

fn machine_identifier() -> io::Result<String> {
    for path in [
        "/etc/machine-id",
        "/var/lib/dbus/machine-id",
        "/sys/class/dmi/id/product_uuid",
    ] {
        if let Ok(value) = fs::read_to_string(path) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Ok(trimmed.to_string());
            }
        }
    }

    if let Ok(output) = Command::new("hostname").output() {
        if output.status.success() {
            if let Ok(text) = String::from_utf8(output.stdout) {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    return Ok(trimmed.to_string());
                }
            }
        }
    }

    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "unable to determine machine identifier",
    ))
}

fn xor_crypt(data: &[u8], key: &[u8; 32], nonce: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(data.len());
    let mut counter = 0u64;
    let mut index = 0usize;

    while index < data.len() {
        let mut hasher = Sha256::new();
        hasher.update(key);
        hasher.update(nonce);
        hasher.update(counter.to_le_bytes());
        let block = hasher.finalize();
        for byte in block {
            if index >= data.len() {
                break;
            }
            output.push(data[index] ^ byte);
            index += 1;
        }
        counter += 1;
    }

    output
}

fn fingerprint_from_public_key(public_key: &[u8]) -> String {
    let hexed = hex::encode(public_key);
    hexed[..16.min(hexed.len())].to_string()
}

fn invalid_identity(error: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_machine_secret_key_is_deterministic_per_machine_id() {
        let first = fallback_machine_secret_key_from_machine_id("machine-a");
        let second = fallback_machine_secret_key_from_machine_id("machine-a");
        let other = fallback_machine_secret_key_from_machine_id("machine-b");

        assert_eq!(first, second);
        assert_ne!(first, other);
        assert_eq!(first.len(), 32);
    }

    #[test]
    fn plaintext_linux_identity_is_rejected() {
        let stored = StoredIdentity {
            public_key_hex: "00".repeat(32),
            fingerprint: "fingerprint".to_string(),
            private_key_hex: "11".repeat(32),
            encrypted_private_key_hex: String::new(),
            nonce_hex: String::new(),
            keychain_label_hex: None,
        };

        let error = reject_plaintext_identity(&stored).expect_err("plaintext should fail");
        assert!(error
            .to_string()
            .contains("plaintext Linux device identity"));
    }
}
