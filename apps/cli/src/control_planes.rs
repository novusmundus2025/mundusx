use serde::{Deserialize, Serialize};
use std::{fs, io, path::Path};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Profile { pub name: String, pub url: String }

#[derive(Debug, Serialize, Deserialize)]
pub struct Registry { pub version: u32, pub control_planes: Vec<Profile> }

impl Default for Registry {
    fn default() -> Self { Self { version: 1, control_planes: Vec::new() } }
}

pub fn path() -> std::path::PathBuf { crate::config::config_dir().join("control-planes.json") }

pub fn normalize(url: &str) -> io::Result<String> {
    let url = url.trim().trim_end_matches('/');
    let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://"));
    if rest.is_none_or(|r| r.split('/').next().unwrap_or_default().is_empty())
        || url.chars().any(char::is_whitespace) || url.contains(['@', '?', '#']) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Use an http:// or https:// endpoint without credentials, query parameters or fragments."));
    }
    Ok(url.into())
}

impl Registry {
    pub fn load_from(path: &Path) -> io::Result<Self> {
        let raw = match fs::read_to_string(path) {
            Ok(raw) => raw,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(e),
        };
        let registry: Self = serde_json::from_str(&raw).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        if registry.version != 1 { return Err(io::Error::new(io::ErrorKind::InvalidData, "Unsupported control-plane registry version")); }
        for profile in &registry.control_planes {
            normalize(&profile.url)?;
            if profile.name.trim().is_empty() { return Err(io::Error::new(io::ErrorKind::InvalidData, "Control-plane name cannot be empty")); }
        }
        Ok(registry)
    }
    pub fn load() -> io::Result<Self> { Self::load_from(&path()) }
    pub fn add(&mut self, name: &str, url: &str) -> io::Result<()> {
        let name = name.trim();
        let url = normalize(url)?;
        if name.is_empty() || name.chars().count() > 80 { return Err(io::Error::new(io::ErrorKind::InvalidInput, "Choose a profile name of 1–80 characters")); }
        if self.control_planes.iter().any(|p| p.url == url) { return Ok(()); }
        if self.control_planes.iter().any(|p| p.name.eq_ignore_ascii_case(name)) {
            return Err(io::Error::new(io::ErrorKind::AlreadyExists, "That profile name is already saved; choose another name or explicitly delete the old profile."));
        }
        self.control_planes.push(Profile { name: name.into(), url });
        Ok(())
    }
    pub fn delete(&mut self, name: &str) -> io::Result<()> {
        let index = self.control_planes.iter().position(|p| p.name.eq_ignore_ascii_case(name))
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Saved control-plane profile not found"))?;
        self.control_planes.remove(index);
        Ok(())
    }
    pub fn save_to(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() { fs::create_dir_all(parent)?; }
        let temp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
        let data = serde_json::to_string_pretty(self).map_err(io::Error::other)?;
        fs::write(&temp, format!("{data}\n"))?;
        if let Err(error) = fs::rename(&temp, path) { let _ = fs::remove_file(temp); return Err(error); }
        Ok(())
    }
    pub fn save(&self) -> io::Result<()> { self.save_to(&path()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saves_multiple_profiles_without_replacing_them_and_deletes_only_explicit_name() {
        let file = std::env::temp_dir().join(format!("mundusx-planes-{}.json", uuid::Uuid::new_v4()));
        let mut registry = Registry::default();
        registry.add("Work", "https://work.example/").unwrap();
        registry.save_to(&file).unwrap();
        let mut registry = Registry::load_from(&file).unwrap();
        registry.add("Home", "http://localhost:8787").unwrap();
        assert!(registry.add("Work", "https://other.example").is_err());
        registry.save_to(&file).unwrap();
        registry.add("Same", "https://work.example").unwrap();
        assert_eq!(registry.control_planes.len(), 2);
        registry.delete("home").unwrap();
        registry.save_to(&file).unwrap();
        assert_eq!(Registry::load_from(&file).unwrap().control_planes[0].url, "https://work.example");
        fs::write(&file, "broken JSON").unwrap();
        assert!(Registry::load_from(&file).is_err());
        assert_eq!(fs::read_to_string(&file).unwrap(), "broken JSON");
        fs::remove_file(file).unwrap();
    }
}
