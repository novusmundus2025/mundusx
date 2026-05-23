#[cfg(target_os = "macos")]
#[path = "../../../tools/macos_identity.rs"]
mod macos_identity;

use crate::storage::{config_dir, identity_path};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeviceIdentity {
    pub public_key_hex: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub private_key_hex: String,
    pub fingerprint: String,
    pub keychain_label_hex: Option<String>,
}

impl DeviceIdentity {
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
        let signature: Signature = signing_key.sign(message.as_bytes());
        Ok(hex::encode(signature.to_bytes()))
    }
}

pub fn load_identity() -> std::io::Result<Option<DeviceIdentity>> {
    #[cfg(target_os = "macos")]
    {
        let identity = macos_secure_identity()?;
        let _ = persist_identity_metadata(&identity)?;
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

pub fn local_identity_path() -> PathBuf {
    PathBuf::from(".opengpu").join("identity.json")
}

fn invalid_identity(error: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, error.to_string())
}

#[cfg(target_os = "macos")]
fn persist_identity_metadata(identity: &DeviceIdentity) -> std::io::Result<PathBuf> {
    let data = serde_json::to_string_pretty(identity).expect("identity serialization");
    if let Some(path) = try_write(&identity_path(), &data)? {
        return Ok(path);
    }
    if let Some(path) = try_write(&local_identity_path(), &data)? {
        return Ok(path);
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "unable to write identity metadata to home or local fallback",
    ))
}

#[cfg(target_os = "macos")]
fn try_write(path: &std::path::Path, data: &str) -> std::io::Result<Option<PathBuf>> {
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

#[cfg(target_os = "macos")]
fn macos_secure_identity() -> std::io::Result<DeviceIdentity> {
    let secure = macos_identity::ensure_identity(&config_dir())?;
    Ok(DeviceIdentity {
        public_key_hex: secure.public_key_hex,
        private_key_hex: String::new(),
        fingerprint: secure.fingerprint,
        keychain_label_hex: Some(secure.keychain_label_hex),
    })
}

#[cfg(target_os = "macos")]
fn macos_sign_hex(message: &str) -> std::io::Result<String> {
    macos_identity::sign_message(&config_dir(), message.as_bytes())
}
