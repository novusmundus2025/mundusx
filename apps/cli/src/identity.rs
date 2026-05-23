#[cfg(target_os = "macos")]
#[path = "../../../tools/macos_identity.rs"]
mod macos_identity;

use crate::config::config_dir;
use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
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
}

impl DeviceIdentity {
    pub fn generate() -> Self {
        #[cfg(target_os = "macos")]
        {
            return macos_secure_identity().expect("macOS secure identity");
        }

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

        let signing_key = self.signing_key()?;
        let signature = signing_key.sign(message.as_bytes());
        Ok(hex::encode(signature.to_bytes()))
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

    let path = resolved_identity_path();
    if !path.exists() {
        return Ok(None);
    }

    let raw = fs::read_to_string(path)?;
    let identity: DeviceIdentity = serde_json::from_str(&raw)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    identity.verifying_key()?;
    if !cfg!(target_os = "macos") || !identity.private_key_hex.trim().is_empty() {
        identity.signing_key()?;
    }
    Ok(Some(identity))
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
    let secure = macos_identity::ensure_identity(&identity_dir())?;
    Ok(DeviceIdentity {
        public_key_hex: secure.public_key_hex,
        private_key_hex: String::new(),
        fingerprint: secure.fingerprint,
        keychain_label_hex: Some(secure.keychain_label_hex),
    })
}

#[cfg(target_os = "macos")]
fn macos_sign_hex(message: &str) -> std::io::Result<String> {
    macos_identity::sign_message(&identity_dir(), message.as_bytes())
}
