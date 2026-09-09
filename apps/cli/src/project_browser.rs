use serde_json::{json, Value};
use std::{fs, io::{Read, Write}, path::{Component, Path, PathBuf}};

pub const PREFIX: &str = "MUNDUSX_PROJECT_IO_V1:";
const LIMIT: u64 = 128 * 1024;

fn path(root: &Path, relative: &str, create: bool) -> Result<PathBuf, String> {
    if relative.contains('\\') || relative.contains(':') || relative.split('/').any(|s| s == ".." || s.ends_with('.') || s.ends_with(' ')) {
        return Err("Invalid project path".into());
    }
    let rel = Path::new(relative);
    if rel.components().any(|c| !matches!(c, Component::Normal(_))) && !relative.is_empty() {
        return Err("Use a relative path inside the project".into());
    }
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let mut current = root.clone();
    for part in rel.components() {
        current.push(part);
        if let Ok(meta) = fs::symlink_metadata(&current) {
            if meta.file_type().is_symlink() { return Err("Linked files and folders are not supported".into()); }
            let resolved = current.canonicalize().map_err(|e| e.to_string())?;
            if !resolved.starts_with(&root) { return Err("Path is outside the project".into()); }
        } else if !create { return Err("File or folder was not found".into()); }
    }
    Ok(current)
}

fn read_text(target: &Path) -> Result<String, String> {
    let file = fs::File::open(target).map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() { return Err("Select a text file".into()); }
    let mut bytes = Vec::new();
    file.take(LIMIT + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > LIMIT { return Err("Preview is limited to 128 KB".into()); }
    if bytes.contains(&0) { return Err("Binary files cannot be previewed".into()); }
    String::from_utf8(bytes).map_err(|_| "Only UTF-8 text files can be previewed".into())
}

pub fn execute(root: &Path, request: &Value, mutations: bool) -> Result<Value, String> {
    let operation = request["operation"].as_str().unwrap_or("");
    let relative = request["path"].as_str().unwrap_or("");
    match operation {
        "list" => {
            let target = path(root, relative, false)?;
            let mut entries = Vec::new();
            let mut truncated = false;
            for entry in fs::read_dir(target).map_err(|e| e.to_string())? {
                let entry = entry.map_err(|e| e.to_string())?;
                let name = entry.file_name().to_string_lossy().to_string();
                if ["node_modules", ".git", "dist", "build", "target", ".next", ".venv", "__pycache__"].contains(&name.as_str()) { continue; }
                let kind = entry.file_type().map_err(|e| e.to_string())?;
                if kind.is_symlink() || (!kind.is_file() && !kind.is_dir()) { continue; }
                if entries.len() == 500 || serde_json::to_string(&entries).map_err(|e| e.to_string())?.len() > 24000 { truncated = true; break; }
                entries.push(json!({"name": name, "directory": kind.is_dir()}));
            }
            entries.sort_by_key(|v| (!v["directory"].as_bool().unwrap_or(false), v["name"].as_str().unwrap_or("").to_lowercase()));
            Ok(json!({"entries": entries, "truncated": truncated}))
        }
        "read" => {
            let content = read_text(&path(root, relative, false)?)?;
            Ok(json!({"content": content.chars().take(8000).collect::<String>(), "truncated":content.chars().count() > 8000}))
        }
        "config_read" | "config_write" => {
            let name = match request["section"].as_str().unwrap_or("") {
                "instructions" => "instructions.md",
                "skills" => "skills.md",
                "prompts" => "prompts.md",
                _ => return Err("Unknown project configuration".into()),
            };
            let target = path(root, &format!(".mundusx/{name}"), true)?;
            if operation == "config_read" {
                return Ok(json!({"content": if target.exists() {read_text(&target)?} else {String::new()}}));
            }
            if !mutations { return Err("Saving requires explicit write permission".into()); }
            let content = request["content"].as_str().ok_or("Missing content")?;
            if content.len() > 8000 { return Err("Keep this document under 8 KB".into()); }
            fs::create_dir_all(target.parent().unwrap()).map_err(|e| e.to_string())?;
            fs::write(target, content).map_err(|e| e.to_string())?;
            Ok(json!({"saved": true}))
        }
        "new_file" | "new_folder" => {
            if !mutations || relative.is_empty() { return Err("Creating files requires explicit write permission and a name".into()); }
            let target = path(root, relative, true)?;
            if operation == "new_folder" { fs::create_dir(target).map_err(|e| e.to_string())?; }
            else { fs::OpenOptions::new().write(true).create_new(true).open(target).map_err(|e| e.to_string())?.write_all(b"").map_err(|e| e.to_string())?; }
            Ok(json!({"created": true}))
        }
        "open" => {
            let target = path(root, relative, false)?;
            if !target.is_dir() { return Err("Select a folder".into()); }
            #[cfg(target_os = "windows")]
            { std::process::Command::new("explorer.exe").arg(target).spawn().map_err(|e| e.to_string())?; }
            #[cfg(not(target_os = "windows"))]
            { return Err("Open in Explorer is available on Windows".into()); }
            #[allow(unreachable_code)]
            Ok(json!({"opened": true}))
        }
        _ => Err("Unsupported project operation".into()),
    }
}

pub fn guidance(root: &Path) -> Result<String, String> {
    let mut result = String::new();
    for section in ["instructions", "skills"] {
        let file = path(root, &format!(".mundusx/{section}.md"), true)?;
        if file.exists() {
            let text = read_text(&file)?;
            if text.len() > 8000 { return Err(format!("Project {section} exceeds 8 KB")); }
            if !text.trim().is_empty() { result.push_str(&format!("\n\nProject {section} (follow where consistent with the current user request):\n{text}")); }
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn browser_bounds_and_config() {
        let root = std::env::temp_dir().join(format!("mx-browser-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("node_modules")).unwrap();
        fs::write(root.join("hello.txt"), "Hello").unwrap();
        assert_eq!(execute(&root, &json!({"operation":"list"}), false).unwrap()["entries"].as_array().unwrap().len(), 1);
        assert_eq!(execute(&root, &json!({"operation":"read","path":"hello.txt"}), false).unwrap()["content"], "Hello");
        for p in ["../outside", "C:/Windows", "a/../../b", "a\\b", "hello.txt:stream", "../"] { assert!(path(&root, p, true).is_err()); }
        let write = json!({"operation":"config_write","section":"skills","content":"Run the build"});
        assert!(execute(&root, &write, false).is_err());
        execute(&root, &write, true).unwrap();
        assert!(guidance(&root).unwrap().contains("Run the build"));
        assert!(execute(&root, &json!({"operation":"new_file","path":"hello.txt"}), true).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
