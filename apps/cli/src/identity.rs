#[cfg(target_os = "macos")]
#[path = "../../../tools/macos_identity.rs"]
mod macos_identity;
#[cfg(windows)]
#[path = "../../../tools/windows_identity.rs"]
mod windows_identity;

use crate::config::config_dir;
#[cfg(all(not(target_os = "macos"), not(windows)))]
use ed25519_dalek::Signer;
use ed25519_dalek::{SigningKey, VerifyingKey};
#[cfg(all(not(target_os = "macos"), not(windows)))]
use rand_core::OsRng;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeviceIdentity {
    pub public_key_hex: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub private_key_hex: String,
    pub fingerprint: String,
    pub keychain_label_hex: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub encrypted_private_key_hex: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub nonce_hex: String,
}

impl DeviceIdentity {
    pub fn generate() -> Self {
        #[cfg(target_os = "macos")]
        {
            return macos_secure_identity().expect("macOS secure identity");
        }

        #[cfg(windows)]
        {
            return windows_secure_identity().expect("Windows secure identity");
        }

        #[cfg(all(not(target_os = "macos"), not(windows)))]
        {
            let signing_key = SigningKey::generate(&mut OsRng);
            let verifying_key = signing_key.verifying_key();
            let public_key_hex = hex::encode(verifying_key.to_bytes());
            let private_key_hex = hex::encode(signing_key.to_bytes());
            let fingerprint = fingerprint_from_public_key(verifying_key.as_bytes());

            Self {
                public_key_hex,
                private_key_hex,
                fingerprint,
                keychain_label_hex: None,
                encrypted_private_key_hex: String::new(),
                nonce_hex: String::new(),
            }
        }
    }

    pub fn signing_key(&self) -> std::io::Result<SigningKey> {
        if self.private_key_hex.trim().is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "private key is stored in secure storage",
            ));
        }

        let private_bytes = hex::decode(&self.private_key_hex).map_err(invalid_identity)?;
        let private_bytes: [u8; 32] = private_bytes.try_into().map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "private key must be 32 bytes",
            )
        })?;
        Ok(SigningKey::from_bytes(&private_bytes))
    }

    pub fn verifying_key(&self) -> std::io::Result<VerifyingKey> {
        let public_bytes = hex::decode(&self.public_key_hex).map_err(invalid_identity)?;
        let public_bytes: [u8; 32] = public_bytes.try_into().map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "public key must be 32 bytes",
            )
        })?;
        VerifyingKey::from_bytes(&public_bytes).map_err(invalid_identity)
    }

    pub fn sign_hex(&self, message: &str) -> std::io::Result<String> {
        #[cfg(target_os = "macos")]
        {
            return macos_sign_hex(message);
        }

        #[cfg(windows)]
        {
            return windows_sign_hex(message);
        }

        #[cfg(all(not(target_os = "macos"), not(windows)))]
        {
            let signing_key = self.signing_key()?;
            let signature = signing_key.sign(message.as_bytes());
            Ok(hex::encode(signature.to_bytes()))
        }
    }
}

pub fn trust_path() -> String {
    #[cfg(target_os = "macos")]
    {
        return macos_identity::trust_path(&macos_storage_dir())
            .unwrap_or("local-encrypted-fallback")
            .to_string();
    }

    #[cfg(windows)]
    {
        windows_identity::trust_path().to_string()
    }

    #[cfg(all(not(target_os = "macos"), not(windows)))]
    {
        "legacy-file".to_string()
    }
}

pub fn identity_dir() -> PathBuf {
    config_dir()
}

pub fn identity_path() -> PathBuf {
    identity_dir().join("identity.json")
}

pub fn local_identity_path() -> PathBuf {
    PathBuf::from(".opengpu").join("identity.json")
}

fn macos_storage_dir() -> PathBuf {
    if std::env::var_os("OPENGPU_HOME").is_some() {
        return identity_dir();
    }

    local_identity_path()
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(".opengpu"))
}

pub fn resolved_identity_path() -> PathBuf {
    if std::env::var_os("OPENGPU_HOME").is_some() {
        return identity_path();
    }

    let home = identity_path();
    if home.exists() {
        return home;
    }

    let local = local_identity_path();
    if local.exists() {
        return local;
    }

    home
}

pub fn load_identity() -> std::io::Result<Option<DeviceIdentity>> {
    #[cfg(target_os = "macos")]
    {
        let identity = macos_secure_identity()?;
        let _ = save_identity(&identity)?;
        return Ok(Some(identity));
    }

    #[cfg(windows)]
    {
        if !resolved_identity_path().exists() {
            return Ok(None);
        }
        let identity = windows_secure_identity()?;
        let _ = save_identity(&identity)?;
        return Ok(Some(identity));
    }

    #[cfg(all(not(target_os = "macos"), not(windows)))]
    {
        let path = resolved_identity_path();
        if !path.exists() {
            return Ok(None);
        }

        let raw = fs::read_to_string(path)?;
        let identity: DeviceIdentity = serde_json::from_str(&raw)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        identity.verifying_key()?;
        identity.signing_key()?;
        Ok(Some(identity))
    }
}

pub fn ensure_identity() -> std::io::Result<(DeviceIdentity, bool, PathBuf)> {
    if let Some(identity) = load_identity()? {
        return Ok((identity, false, resolved_identity_path()));
    }

    let identity = DeviceIdentity::generate();
    let path = save_identity(&identity)?;
    Ok((identity, true, path))
}

pub fn save_identity(identity: &DeviceIdentity) -> std::io::Result<PathBuf> {
    let data = serde_json::to_string_pretty(identity).expect("identity serialization");

    if let Some(path) = try_write(&identity_path(), &data)? {
        return Ok(path);
    }

    if let Some(path) = try_write(&local_identity_path(), &data)? {
        return Ok(path);
    }

    Err(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "unable to write identity to home or local fallback",
    ))
}

pub fn load_or_create_identity() -> std::io::Result<(DeviceIdentity, bool, PathBuf)> {
    ensure_identity()
}

pub fn fingerprint_from_public_key(public_key: &[u8]) -> String {
    let hexed = hex::encode(public_key);
    hexed[..16.min(hexed.len())].to_string()
}

pub fn device_id_for_identity(identity: &DeviceIdentity) -> String {
    format!("node-{}", identity.fingerprint)
}

fn try_write(path: &Path, data: &str) -> std::io::Result<Option<PathBuf>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    match fs::write(path, format!("{data}\n")) {
        Ok(()) => Ok(Some(path.to_path_buf())),
        Err(error) => {
            if path == identity_path() {
                Ok(None)
            } else {
                Err(error)
            }
        }
    }
}

fn invalid_identity(error: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, error.to_string())
}

#[cfg(target_os = "macos")]
fn macos_secure_identity() -> std::io::Result<DeviceIdentity> {
    let secure = macos_identity::ensure_identity(&macos_storage_dir())?;
    Ok(DeviceIdentity {
        public_key_hex: secure.public_key_hex,
        private_key_hex: String::new(),
        fingerprint: secure.fingerprint,
        keychain_label_hex: Some(secure.keychain_label_hex),
        encrypted_private_key_hex: secure.encrypted_private_key_hex,
        nonce_hex: secure.nonce_hex,
    })
}

#[cfg(target_os = "macos")]
fn macos_sign_hex(message: &str) -> std::io::Result<String> {
    macos_identity::sign_message(&macos_storage_dir(), message.as_bytes())
}

#[cfg(windows)]
fn windows_storage_dir() -> PathBuf {
    if std::env::var_os("OPENGPU_HOME").is_some() {
        return identity_dir();
    }

    let home = identity_dir();
    if identity_path().exists() || !local_identity_path().exists() {
        return home;
    }

    local_identity_path()
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(".opengpu"))
}

#[cfg(windows)]
fn windows_secure_identity() -> std::io::Result<DeviceIdentity> {
    let secure = windows_identity::ensure_identity(&windows_storage_dir())?;
    Ok(DeviceIdentity {
        public_key_hex: secure.public_key_hex,
        private_key_hex: String::new(),
        fingerprint: secure.fingerprint,
        keychain_label_hex: None,
        encrypted_private_key_hex: secure.encrypted_private_key_hex,
        nonce_hex: String::new(),
    })
}

#[cfg(windows)]
fn windows_sign_hex(message: &str) -> std::io::Result<String> {
    windows_identity::sign_message(&windows_storage_dir(), message.as_bytes())
}
