use crate::storage::identity_path;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeviceIdentity {
    pub public_key_hex: String,
    pub private_key_hex: String,
    pub fingerprint: String,
}

pub fn load_identity() -> std::io::Result<Option<DeviceIdentity>> {
    let path = resolved_identity_path();
    if !path.exists() {
        return Ok(None);
    }

    let raw = fs::read_to_string(path)?;
    let identity: DeviceIdentity = serde_json::from_str(&raw)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
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
