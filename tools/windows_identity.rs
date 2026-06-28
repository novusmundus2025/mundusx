use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
use rand_core::OsRng;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use windows_sys::Win32::Security::Cryptography::{
    CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
};

const TRUST_PATH: &str = "dpapi://mundusx/device-identity";

#[derive(Debug, Deserialize)]
struct StoredIdentity {
    public_key_hex: String,
    fingerprint: String,
    #[serde(default)]
    private_key_hex: String,
    #[serde(default)]
    encrypted_private_key_hex: String,
}

#[derive(Debug, Serialize)]
struct PersistedIdentity {
    public_key_hex: String,
    fingerprint: String,
    keychain_label_hex: Option<String>,
    encrypted_private_key_hex: String,
}

#[derive(Clone, Debug)]
pub struct SecureIdentity {
    pub public_key_hex: String,
    pub fingerprint: String,
    pub encrypted_private_key_hex: String,
}

pub fn ensure_identity(storage_dir: &Path) -> io::Result<SecureIdentity> {
    if let Some(existing) = load_stored_identity(storage_dir)? {
        if !existing.private_key_hex.trim().is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "plaintext Windows device identity is no longer accepted; remove identity.json and re-enroll",
            ));
        }
        if existing.encrypted_private_key_hex.trim().is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Windows device identity is missing DPAPI-protected key material",
            ));
        }
        existing.verifying_key()?;
        let signature = sign_message(storage_dir, b"identity self-check")?;
        verify_message(&existing.public_key_hex, &signature, b"identity self-check")?;
        return Ok(SecureIdentity {
            public_key_hex: existing.public_key_hex,
            fingerprint: existing.fingerprint,
            encrypted_private_key_hex: existing.encrypted_private_key_hex,
        });
    }

    let signing_key = SigningKey::generate(&mut OsRng);
    let verifying_key = signing_key.verifying_key();
    let public_key_hex = hex::encode(verifying_key.to_bytes());
    let fingerprint = fingerprint_from_public_key(verifying_key.as_bytes());
    let encrypted_private_key_hex = hex::encode(protect_private_key(&signing_key.to_bytes())?);
    let persisted = PersistedIdentity {
        public_key_hex: public_key_hex.clone(),
        fingerprint: fingerprint.clone(),
        keychain_label_hex: None,
        encrypted_private_key_hex: encrypted_private_key_hex.clone(),
    };
    save_stored_identity(storage_dir, &persisted)?;

    Ok(SecureIdentity {
        public_key_hex,
        fingerprint,
        encrypted_private_key_hex,
    })
}

pub fn sign_message(storage_dir: &Path, message: &[u8]) -> io::Result<String> {
    let stored = load_stored_identity(storage_dir)?.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "missing Windows device identity; run `opengpu start` first",
        )
    })?;
    if !stored.private_key_hex.trim().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "plaintext Windows device identity is no longer accepted; remove identity.json and re-enroll",
        ));
    }
    if stored.encrypted_private_key_hex.trim().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Windows device identity is missing DPAPI-protected key material",
        ));
    }

    let encrypted = hex::decode(&stored.encrypted_private_key_hex).map_err(invalid_identity)?;
    let private_bytes = unprotect_private_key(&encrypted)?;
    let private_bytes: [u8; 32] = private_bytes
        .try_into()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "private key must be 32 bytes"))?;
    let signing_key = SigningKey::from_bytes(&private_bytes);
    let signature = signing_key.sign(message);
    Ok(hex::encode(signature.to_bytes()))
}

pub fn verify_message(public_key_hex: &str, signature_hex: &str, message: &[u8]) -> io::Result<()> {
    let public_bytes = hex::decode(public_key_hex).map_err(invalid_identity)?;
    let public_bytes: [u8; 32] = public_bytes
        .try_into()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "public key must be 32 bytes"))?;
    let verifying_key = VerifyingKey::from_bytes(&public_bytes).map_err(invalid_identity)?;
    let signature_bytes = hex::decode(signature_hex).map_err(invalid_identity)?;
    let signature =
        ed25519_dalek::Signature::from_slice(&signature_bytes).map_err(invalid_identity)?;
    verifying_key
        .verify(message, &signature)
        .map_err(invalid_identity)
}

pub fn trust_path() -> &'static str {
    TRUST_PATH
}

fn load_stored_identity(storage_dir: &Path) -> io::Result<Option<StoredIdentity>> {
    let path = identity_path(storage_dir);
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(path)?;
    serde_json::from_str(&raw)
        .map(Some)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn save_stored_identity(storage_dir: &Path, identity: &PersistedIdentity) -> io::Result<()> {
    let path = identity_path(storage_dir);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let data = serde_json::to_string_pretty(identity).expect("identity serialization");
    fs::write(path, format!("{data}\n"))
}

fn identity_path(storage_dir: &Path) -> PathBuf {
    storage_dir.join("identity.json")
}

fn protect_private_key(private_key: &[u8; 32]) -> io::Result<Vec<u8>> {
    let input = CRYPT_INTEGER_BLOB {
        cbData: private_key.len() as u32,
        pbData: private_key.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    let ok = unsafe {
        CryptProtectData(
            &input,
            std::ptr::null(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    let encrypted =
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
    unsafe {
        windows_sys::Win32::Foundation::LocalFree(output.pbData as _);
    }
    Ok(encrypted)
}

fn unprotect_private_key(encrypted: &[u8]) -> io::Result<Vec<u8>> {
    let input = CRYPT_INTEGER_BLOB {
        cbData: encrypted.len() as u32,
        pbData: encrypted.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    let ok = unsafe {
        CryptUnprotectData(
            &input,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    let private_key =
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
    unsafe {
        windows_sys::Win32::Foundation::LocalFree(output.pbData as _);
    }
    Ok(private_key)
}

fn fingerprint_from_public_key(public_key: &[u8]) -> String {
    let hexed = hex::encode(public_key);
    hexed[..16.min(hexed.len())].to_string()
}

impl StoredIdentity {
    fn verifying_key(&self) -> io::Result<VerifyingKey> {
        let public_bytes = hex::decode(&self.public_key_hex).map_err(invalid_identity)?;
        let public_bytes: [u8; 32] = public_bytes.try_into().map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidData, "public key must be 32 bytes")
        })?;
        VerifyingKey::from_bytes(&public_bytes).map_err(invalid_identity)
    }
}

fn invalid_identity(error: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_identity_dir(name: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "opengpu-windows-identity-{name}-{}-{suffix}",
            std::process::id()
        ))
    }

    #[test]
    fn dpapi_identity_omits_plaintext_key_and_signs() {
        let dir = temp_identity_dir("protected");
        let identity = ensure_identity(&dir).expect("identity");
        assert!(!identity.encrypted_private_key_hex.is_empty());

        let raw = fs::read_to_string(identity_path(&dir)).expect("identity json");
        let json: serde_json::Value = serde_json::from_str(&raw).expect("json");
        assert!(json.get("private_key_hex").is_none());
        assert_eq!(
            json.get("encrypted_private_key_hex")
                .and_then(serde_json::Value::as_str),
            Some(identity.encrypted_private_key_hex.as_str())
        );

        let message = b"register heartbeat claim complete";
        let signature = sign_message(&dir, message).expect("signature");
        verify_message(&identity.public_key_hex, &signature, message).expect("verified");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn plaintext_windows_identity_is_rejected() {
        let dir = temp_identity_dir("legacy");
        fs::create_dir_all(&dir).expect("dir");
        let signing_key = SigningKey::generate(&mut OsRng);
        let verifying_key = signing_key.verifying_key();
        let public_key_hex = hex::encode(verifying_key.to_bytes());
        let legacy = json!({
            "public_key_hex": public_key_hex,
            "private_key_hex": hex::encode(signing_key.to_bytes()),
            "fingerprint": fingerprint_from_public_key(verifying_key.as_bytes()),
            "keychain_label_hex": null
        });
        fs::write(identity_path(&dir), format!("{legacy}\n")).expect("legacy identity");

        let error = ensure_identity(&dir).expect_err("legacy identity must fail");
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(error
            .to_string()
            .contains("plaintext Windows device identity"));

        let _ = fs::remove_dir_all(&dir);
    }
}
