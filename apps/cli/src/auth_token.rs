use crate::config::{config_dir, Config};
use std::io;
use std::path::PathBuf;

#[cfg(windows)]
const DPAPI_TOKEN_FILE: &str = "operator-token.dpapi";

#[cfg(windows)]
pub fn protected_token_path() -> PathBuf {
    config_dir().join(DPAPI_TOKEN_FILE)
}

pub fn effective_operator_token(config: &Config) -> Option<String> {
    load_operator_token()
        .ok()
        .flatten()
        .or_else(|| config.auth_token.clone())
        .filter(|token| !token.trim().is_empty())
}

pub fn operator_token_present(config: &Config) -> bool {
    effective_operator_token(config).is_some()
}

#[cfg(windows)]
pub fn store_operator_token(token: &str) -> io::Result<PathBuf> {
    use std::fs;
    use windows_sys::Win32::Security::Cryptography::{
        CryptProtectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    let token_bytes = token.as_bytes();
    let input = CRYPT_INTEGER_BLOB {
        cbData: token_bytes.len() as u32,
        pbData: token_bytes.as_ptr() as *mut u8,
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

    let path = protected_token_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, encrypted)?;
    Ok(path)
}

#[cfg(not(windows))]
pub fn store_operator_token(_token: &str) -> io::Result<PathBuf> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "protected operator token storage is only available on Windows",
    ))
}

#[cfg(windows)]
pub fn load_operator_token() -> io::Result<Option<String>> {
    use std::fs;
    use windows_sys::Win32::Security::Cryptography::{
        CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    let path = protected_token_path();
    if !path.exists() {
        return Ok(None);
    }

    let encrypted = fs::read(path)?;
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

    let token_bytes =
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
    unsafe {
        windows_sys::Win32::Foundation::LocalFree(output.pbData as _);
    }

    String::from_utf8(token_bytes)
        .map(Some)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

#[cfg(not(windows))]
pub fn load_operator_token() -> io::Result<Option<String>> {
    Ok(None)
}

#[cfg(windows)]
pub fn clear_operator_token() -> io::Result<()> {
    match std::fs::remove_file(protected_token_path()) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(not(windows))]
pub fn clear_operator_token() -> io::Result<()> {
    Ok(())
}
