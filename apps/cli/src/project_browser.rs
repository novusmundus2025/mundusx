use serde_json::{json, Value};
use std::{
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    process::Command,
};

pub const PREFIX: &str = "MUNDUSX_PROJECT_IO_V1:";
const LIMIT: u64 = 128 * 1024;

fn path(root: &Path, relative: &str, create: bool) -> Result<PathBuf, String> {
    if relative.contains('\\')
        || relative.contains(':')
        || relative
            .split('/')
            .any(|s| s == ".." || s.ends_with('.') || s.ends_with(' '))
    {
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
            if meta.file_type().is_symlink() {
                return Err("Linked files and folders are not supported".into());
            }
            let resolved = current.canonicalize().map_err(|e| e.to_string())?;
            if !resolved.starts_with(&root) {
                return Err("Path is outside the project".into());
            }
        } else if !create {
            return Err("File or folder was not found".into());
        }
    }
    Ok(current)
}

fn read_text(target: &Path) -> Result<String, String> {
    let file = fs::File::open(target).map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("Select a text file".into());
    }
    let mut bytes = Vec::new();
    file.take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > LIMIT {
        return Err("Preview is limited to 128 KB".into());
    }
    if bytes.contains(&0) {
        return Err("Binary files cannot be previewed".into());
    }
    String::from_utf8(bytes).map_err(|_| "Only UTF-8 text files can be previewed".into())
}

fn valid_git_ref(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 200
        && !value.starts_with('-')
        && !value.contains("..")
        && !value.contains("//")
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "._/-".contains(character))
}

fn valid_github_remote(value: &str) -> bool {
    let https = value.strip_prefix("https://github.com/");
    let ssh = value.strip_prefix("git@github.com:");
    let Some(repository) = https.or(ssh) else {
        return false;
    };
    let repository = repository.strip_suffix(".git").unwrap_or(repository);
    let mut parts = repository.split('/');
    let owner = parts.next().unwrap_or("");
    let name = parts.next().unwrap_or("");
    parts.next().is_none()
        && [owner, name].iter().all(|part| {
            !part.is_empty()
                && part.len() <= 100
                && !part.starts_with('-')
                && !part.ends_with('.')
                && part
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || ".-_".contains(character))
        })
}

fn git(root: &Path, args: &[&str]) -> Result<String, String> {
    let disabled_hooks = std::env::temp_dir().join("mundusx-disabled-git-hooks");
    fs::create_dir_all(&disabled_hooks)
        .map_err(|error| format!("Could not prepare safe Git execution: {error}"))?;
    let output = Command::new("git")
        .arg("-c")
        .arg(format!("core.hooksPath={}", disabled_hooks.display()))
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|error| format!("Git is not installed or could not be started: {error}"))?;
    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr)
            .trim()
            .chars()
            .take(4000)
            .collect::<String>();
        return Err(if error.is_empty() {
            "Git operation failed".into()
        } else {
            error
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .trim()
        .chars()
        .take(24_000)
        .collect())
}

fn git_initialized(root: &Path) -> bool {
    git(root, &["rev-parse", "--is-inside-work-tree"])
        .map(|value| value == "true")
        .unwrap_or(false)
}

fn git_status(root: &Path) -> Result<Value, String> {
    if !git_initialized(root) {
        return Ok(
            json!({"initialized": false, "branch": null, "remote_url": null, "changed_files": 0, "changes": []}),
        );
    }
    let branch = git(root, &["branch", "--show-current"])?;
    let remote_url = git(root, &["remote", "get-url", "origin"])
        .ok()
        .filter(|value| !value.is_empty());
    let porcelain = git(root, &["status", "--porcelain=v1", "--branch"])?;
    let mut ahead = 0_u64;
    let mut behind = 0_u64;
    let mut changes = Vec::new();
    for line in porcelain.lines() {
        if let Some(summary) = line.strip_prefix("## ") {
            if let Some(value) = summary
                .split("ahead ")
                .nth(1)
                .and_then(|value| {
                    value
                        .split(|character: char| !character.is_ascii_digit())
                        .next()
                })
                .and_then(|value| value.parse().ok())
            {
                ahead = value;
            }
            if let Some(value) = summary
                .split("behind ")
                .nth(1)
                .and_then(|value| {
                    value
                        .split(|character: char| !character.is_ascii_digit())
                        .next()
                })
                .and_then(|value| value.parse().ok())
            {
                behind = value;
            }
        } else if line.len() >= 3 {
            changes.push(json!({"status": &line[..2], "path": line[3..].to_string()}));
        }
    }
    let has_commits = git(root, &["rev-parse", "--verify", "HEAD"]).is_ok();
    Ok(json!({
        "initialized": true,
        "branch": if branch.is_empty() { "main" } else { branch.as_str() },
        "remote_url": remote_url,
        "changed_files": changes.len(),
        "changes": changes,
        "ahead": ahead,
        "behind": behind,
        "has_commits": has_commits,
    }))
}

pub fn execute(root: &Path, request: &Value, mutations: bool) -> Result<Value, String> {
    let operation = request["operation"].as_str().unwrap_or("");
    let relative = request["path"].as_str().unwrap_or("");
    match operation {
        "ensure_project" => {
            if !mutations {
                return Err("Creating a project requires explicit write permission".into());
            }
            Ok(json!({"created": true}))
        }
        "git_status" => git_status(root),
        "git_init" => {
            if !mutations {
                return Err("Initializing Git requires explicit write permission".into());
            }
            if !git_initialized(root) {
                if git(root, &["init", "-b", "main"]).is_err() {
                    git(root, &["init"])?;
                    let _ = git(root, &["branch", "-M", "main"]);
                }
            }
            let author_name = request["author_name"].as_str().unwrap_or("").trim();
            let author_email = request["author_email"].as_str().unwrap_or("").trim();
            if !author_name.is_empty()
                && author_name.len() <= 200
                && !author_name.chars().any(char::is_control)
            {
                git(root, &["config", "--local", "user.name", author_name])?;
            }
            if !author_email.is_empty()
                && author_email.len() <= 254
                && author_email.contains('@')
                && !author_email.chars().any(char::is_control)
            {
                git(root, &["config", "--local", "user.email", author_email])?;
            }
            git_status(root)
        }
        "git_clone" => {
            if !mutations {
                return Err("Cloning requires explicit write permission".into());
            }
            if git_initialized(root)
                || fs::read_dir(root)
                    .map_err(|error| error.to_string())?
                    .next()
                    .is_some()
            {
                return Err("Clone requires an empty project folder".into());
            }
            let remote = request["url"].as_str().unwrap_or("").trim();
            if !valid_github_remote(remote) {
                return Err("Only a repository URL on github.com can be cloned".into());
            }
            git(root, &["clone", "--origin", "origin", "--", remote, "."])?;
            git_status(root)
        }
        "git_commit" => {
            if !mutations || !git_initialized(root) {
                return Err(
                    "Committing requires an initialized Git project and explicit write permission"
                        .into(),
                );
            }
            let message = request["message"].as_str().unwrap_or("").trim();
            if message.is_empty() || message.len() > 200 || message.chars().any(char::is_control) {
                return Err("Commit message must contain 1 to 200 characters".into());
            }
            git(root, &["add", "--all", "--", "."])?;
            if git(root, &["diff", "--cached", "--quiet"]).is_ok() {
                return Ok(
                    json!({"committed": false, "message": "No changes to commit", "status": git_status(root)?}),
                );
            }
            git(root, &["commit", "-m", message])?;
            Ok(json!({"committed": true, "status": git_status(root)?}))
        }
        "git_remote_set" => {
            if !mutations || !git_initialized(root) {
                return Err("Connecting a remote requires an initialized Git project and explicit write permission".into());
            }
            let remote = request["url"].as_str().unwrap_or("").trim();
            if !valid_github_remote(remote) {
                return Err("Only a repository URL on github.com can be connected".into());
            }
            if git(root, &["remote", "get-url", "origin"]).is_ok() {
                git(root, &["remote", "set-url", "origin", remote])?;
            } else {
                git(root, &["remote", "add", "origin", remote])?;
            }
            git_status(root)
        }
        "git_fetch" => {
            if !mutations || !git_initialized(root) {
                return Err(
                    "Fetching requires an initialized Git project and explicit write permission"
                        .into(),
                );
            }
            git(root, &["fetch", "--prune", "origin"])?;
            git_status(root)
        }
        "git_update" => {
            if !mutations || !git_initialized(root) {
                return Err(
                    "Updating requires an initialized Git project and explicit write permission"
                        .into(),
                );
            }
            git(root, &["pull", "--ff-only", "origin"])?;
            git_status(root)
        }
        "git_create_branch" => {
            if !mutations || !git_initialized(root) {
                return Err("Creating a branch requires an initialized Git project and explicit write permission".into());
            }
            let branch = request["branch"].as_str().unwrap_or("").trim();
            if !valid_git_ref(branch) {
                return Err("Branch name is invalid".into());
            }
            git(root, &["switch", "-c", branch])?;
            git_status(root)
        }
        "git_push" => {
            if !mutations || !git_initialized(root) {
                return Err(
                    "Pushing requires an initialized Git project and explicit write permission"
                        .into(),
                );
            }
            let status = git_status(root)?;
            let branch = request["branch"]
                .as_str()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| status["branch"].as_str().unwrap_or("main"));
            if !valid_git_ref(branch) {
                return Err("Branch name is invalid".into());
            }
            git(
                root,
                &[
                    "push",
                    "--set-upstream",
                    "origin",
                    &format!("HEAD:refs/heads/{branch}"),
                ],
            )?;
            git_status(root)
        }
        "list" => {
            let target = path(root, relative, false)?;
            let mut entries = Vec::new();
            let mut truncated = false;
            for entry in fs::read_dir(target).map_err(|e| e.to_string())? {
                let entry = entry.map_err(|e| e.to_string())?;
                let name = entry.file_name().to_string_lossy().to_string();
                if [
                    "node_modules",
                    ".git",
                    "dist",
                    "build",
                    "target",
                    ".next",
                    ".venv",
                    "__pycache__",
                ]
                .contains(&name.as_str())
                {
                    continue;
                }
                let kind = entry.file_type().map_err(|e| e.to_string())?;
                if kind.is_symlink() || (!kind.is_file() && !kind.is_dir()) {
                    continue;
                }
                if entries.len() == 500
                    || serde_json::to_string(&entries)
                        .map_err(|e| e.to_string())?
                        .len()
                        > 24000
                {
                    truncated = true;
                    break;
                }
                entries.push(json!({"name": name, "directory": kind.is_dir()}));
            }
            entries.sort_by_key(|v| {
                (
                    !v["directory"].as_bool().unwrap_or(false),
                    v["name"].as_str().unwrap_or("").to_lowercase(),
                )
            });
            Ok(json!({"entries": entries, "truncated": truncated}))
        }
        "read" => {
            let content = read_text(&path(root, relative, false)?)?;
            Ok(
                json!({"content": content.chars().take(8000).collect::<String>(), "truncated":content.chars().count() > 8000}),
            )
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
                return Ok(
                    json!({"content": if target.exists() {read_text(&target)?} else {String::new()}}),
                );
            }
            if !mutations {
                return Err("Saving requires explicit write permission".into());
            }
            let content = request["content"].as_str().ok_or("Missing content")?;
            if content.len() > 8000 {
                return Err("Keep this document under 8 KB".into());
            }
            fs::create_dir_all(target.parent().unwrap()).map_err(|e| e.to_string())?;
            fs::write(target, content).map_err(|e| e.to_string())?;
            Ok(json!({"saved": true}))
        }
        "new_file" | "new_folder" => {
            if !mutations || relative.is_empty() {
                return Err("Creating files requires explicit write permission and a name".into());
            }
            let target = path(root, relative, true)?;
            if operation == "new_folder" {
                fs::create_dir(target).map_err(|e| e.to_string())?;
            } else {
                fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(target)
                    .map_err(|e| e.to_string())?
                    .write_all(b"")
                    .map_err(|e| e.to_string())?;
            }
            Ok(json!({"created": true}))
        }
        "open" => {
            let target = path(root, relative, false)?;
            if !target.is_dir() {
                return Err("Select a folder".into());
            }
            #[cfg(target_os = "windows")]
            {
                std::process::Command::new("explorer.exe")
                    .arg(target)
                    .spawn()
                    .map_err(|e| e.to_string())?;
            }
            #[cfg(target_os = "macos")]
            {
                std::process::Command::new("open")
                    .arg(target)
                    .spawn()
                    .map_err(|e| e.to_string())?;
            }
            #[cfg(target_os = "linux")]
            {
                std::process::Command::new("xdg-open")
                    .arg(target)
                    .spawn()
                    .map_err(|e| e.to_string())?;
            }
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
            if text.len() > 8000 {
                return Err(format!("Project {section} exceeds 8 KB"));
            }
            if !text.trim().is_empty() {
                result.push_str(&format!("\n\nProject {section} (follow where consistent with the current user request):\n{text}"));
            }
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
        assert_eq!(
            execute(&root, &json!({"operation":"list"}), false).unwrap()["entries"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            execute(
                &root,
                &json!({"operation":"read","path":"hello.txt"}),
                false
            )
            .unwrap()["content"],
            "Hello"
        );
        for p in [
            "../outside",
            "C:/Windows",
            "a/../../b",
            "a\\b",
            "hello.txt:stream",
            "../",
        ] {
            assert!(path(&root, p, true).is_err());
        }
        let write =
            json!({"operation":"config_write","section":"skills","content":"Run the build"});
        assert!(execute(&root, &write, false).is_err());
        execute(&root, &write, true).unwrap();
        assert!(guidance(&root).unwrap().contains("Run the build"));
        assert!(execute(
            &root,
            &json!({"operation":"new_file","path":"hello.txt"}),
            true
        )
        .is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn git_operations_are_bounded_to_the_project() {
        if Command::new("git")
            .arg("--version")
            .output()
            .map(|output| !output.status.success())
            .unwrap_or(true)
        {
            return;
        }
        let root = std::env::temp_dir().join(format!("mx-browser-git-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let initialized = execute(
            &root,
            &json!({
                "operation":"git_init",
                "author_name":"MundusX Test",
                "author_email":"mundusx-test@example.com"
            }),
            true,
        )
        .unwrap();
        assert_eq!(initialized["initialized"], true);
        fs::write(root.join("hello.txt"), "Hello").unwrap();
        let committed = execute(
            &root,
            &json!({"operation":"git_commit","message":"Create hello file"}),
            true,
        )
        .unwrap();
        assert_eq!(committed["committed"], true);
        assert!(execute(
            &root,
            &json!({"operation":"git_remote_set","url":"https://example.com/owner/repo.git"}),
            true
        )
        .is_err());
        let connected = execute(
            &root,
            &json!({"operation":"git_remote_set","url":"https://github.com/owner/repo.git"}),
            true,
        )
        .unwrap();
        assert_eq!(connected["remote_url"], "https://github.com/owner/repo.git");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn clone_rejects_untrusted_remotes_and_non_empty_projects() {
        let root = std::env::temp_dir().join(format!("mx-browser-clone-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        assert!(execute(
            &root,
            &json!({"operation":"git_clone","url":"https://example.com/owner/repo.git"}),
            true
        )
        .is_err());
        fs::write(root.join("keep.txt"), "keep").unwrap();
        assert!(execute(
            &root,
            &json!({"operation":"git_clone","url":"https://github.com/owner/repo.git"}),
            true
        )
        .is_err());
        assert_eq!(fs::read_to_string(root.join("keep.txt")).unwrap(), "keep");
        fs::remove_dir_all(root).unwrap();
    }
}
