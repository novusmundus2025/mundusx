use crate::storage::identity_path;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeviceIdentity {
    pub public_key_hex: String,
    pub private_key_hex: String,
    pub fingerprint: String,
}

impl DeviceIdentity {
    pub fn signing_key(&self) -> std::io::Result<SigningKey> {
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
        let signing_key = self.signing_key()?;
        let signature: Signature = signing_key.sign(message.as_bytes());
        Ok(hex::encode(signature.to_bytes()))
    }
}

pub fn load_identity() -> std::io::Result<Option<DeviceIdentity>> {
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
