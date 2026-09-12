use sha2::{Digest, Sha256};
use std::env;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

const RELEASES_API_URL: &str = "https://api.github.com/repos/mundusx/releases/releases?per_page=50";

struct StagedBinary {
    path: PathBuf,
    target: PathBuf,
}

struct ReleaseAssets {
    tag: String,
    cli_url: String,
    cli_checksum_url: String,
    agent_url: String,
    agent_checksum_url: String,
}

impl Drop for StagedBinary {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

pub fn update_installed_binaries() -> Result<(), String> {
    let target = release_target(env::consts::OS, env::consts::ARCH)?;
    let current_exe =
        env::current_exe().map_err(|error| format!("cannot locate installed opengpu: {error}"))?;
    let install_dir = current_exe
        .parent()
        .ok_or_else(|| "installed opengpu path has no parent directory".to_string())?;
    let assets = resolve_release_assets(target)?;

    println!("updateChannel: linux");
    println!("releaseTag: {}", assets.tag);
    println!("releaseTarget: {target}");
    println!("updateStage: downloading and verifying");

    let cli = stage_release_binary(
        &format!("opengpu-{target}"),
        &assets.cli_url,
        &assets.cli_checksum_url,
        &current_exe,
    )?;
    let agent_target = install_dir.join("opengpu-node-agent");
    let agent = stage_release_binary(
        &format!("opengpu-node-agent-{target}"),
        &assets.agent_url,
        &assets.agent_checksum_url,
        &agent_target,
    )?;

    smoke_check(&cli.path, "opengpu")?;
    smoke_check(&agent.path, "opengpu-node-agent")?;

    // Replace the companion first so the final CLI swap is the commit point.
    replace_binary(&agent.path, &agent.target)?;
    replace_binary(&cli.path, &cli.target)?;

    println!("updateStage: complete");
    println!("updated: {}", cli.target.display());
    println!("updatedAgent: {}", agent.target.display());
    println!("updateHint: restart the node agent to use the new version");
    Ok(())
}

fn resolve_release_assets(target: &str) -> Result<ReleaseAssets, String> {
    let cli_name = format!("opengpu-{target}");
    let agent_name = format!("opengpu-node-agent-{target}");
    if let Ok(base_url) = env::var("OPENGPU_RELEASE_BASE_URL") {
        let base = base_url.trim_end_matches('/');
        return Ok(ReleaseAssets {
            tag: "custom".to_string(),
            cli_url: format!("{base}/{cli_name}"),
            cli_checksum_url: format!("{base}/{cli_name}.sha256"),
            agent_url: format!("{base}/{agent_name}"),
            agent_checksum_url: format!("{base}/{agent_name}.sha256"),
        });
    }

    let releases = String::from_utf8(download(RELEASES_API_URL)?)
        .map_err(|_| "GitHub releases API returned invalid UTF-8".to_string())?;
    parse_linux_release_assets(&releases, &cli_name, &agent_name)
}

fn parse_linux_release_assets(
    releases_json: &str,
    cli_name: &str,
    agent_name: &str,
) -> Result<ReleaseAssets, String> {
    let releases: serde_json::Value = serde_json::from_str(releases_json)
        .map_err(|error| format!("invalid GitHub releases response: {error}"))?;
    let releases = releases
        .as_array()
        .ok_or_else(|| "GitHub releases response was not an array".to_string())?;
    for release in releases {
        let tag = release["tag_name"].as_str().unwrap_or_default();
        if !tag.starts_with("cli-linux-v")
            || release["draft"].as_bool().unwrap_or(false)
            || release["prerelease"].as_bool().unwrap_or(false)
        {
            continue;
        }
        let assets = release["assets"]
            .as_array()
            .ok_or_else(|| format!("release {tag} has no assets"))?;
        let asset_url = |name: &str| {
            assets.iter().find_map(|asset| {
                (asset["name"].as_str() == Some(name))
                    .then(|| asset["browser_download_url"].as_str())
                    .flatten()
                    .map(str::to_string)
            })
        };
        return Ok(ReleaseAssets {
            tag: tag.to_string(),
            cli_url: asset_url(cli_name)
                .ok_or_else(|| format!("release {tag} is missing {cli_name}"))?,
            cli_checksum_url: asset_url(&format!("{cli_name}.sha256"))
                .ok_or_else(|| format!("release {tag} is missing {cli_name}.sha256"))?,
            agent_url: asset_url(agent_name)
                .ok_or_else(|| format!("release {tag} is missing {agent_name}"))?,
            agent_checksum_url: asset_url(&format!("{agent_name}.sha256"))
                .ok_or_else(|| format!("release {tag} is missing {agent_name}.sha256"))?,
        });
    }
    Err("no published cli-linux-v* release was found".to_string())
}

fn release_target(os: &str, arch: &str) -> Result<&'static str, String> {
    match (os, arch) {
        ("linux", "aarch64" | "arm64") => Ok("aarch64-unknown-linux-gnu"),
        ("linux", "x86_64" | "amd64") => Ok("x86_64-unknown-linux-gnu"),
        ("linux", _) => Err(format!("unsupported Linux architecture: {arch}")),
        _ => Err("opengpu self-update currently supports Linux only".to_string()),
    }
}

fn stage_release_binary(
    asset_name: &str,
    asset_url: &str,
    checksum_url: &str,
    target: &Path,
) -> Result<StagedBinary, String> {
    let bytes = download(asset_url)?;
    let checksum = String::from_utf8(download(checksum_url)?)
        .map_err(|_| format!("invalid UTF-8 checksum for {asset_name}"))?;
    verify_checksum(asset_name, &bytes, &checksum)?;

    let parent = target
        .parent()
        .ok_or_else(|| format!("update target has no parent: {}", target.display()))?;
    fs::create_dir_all(parent).map_err(|error| {
        format!(
            "cannot create install directory {}: {error}",
            parent.display()
        )
    })?;
    let staged_path = parent.join(format!(".{asset_name}.update-{}", std::process::id()));
    let mut staged = fs::File::create(&staged_path)
        .map_err(|error| format!("cannot stage {}: {error}", staged_path.display()))?;
    staged
        .write_all(&bytes)
        .map_err(|error| format!("cannot write {}: {error}", staged_path.display()))?;
    staged
        .sync_all()
        .map_err(|error| format!("cannot sync {}: {error}", staged_path.display()))?;
    set_executable(&staged_path)?;

    Ok(StagedBinary {
        path: staged_path,
        target: target.to_path_buf(),
    })
}

fn download(url: &str) -> Result<Vec<u8>, String> {
    let mut request = ureq::get(url).set("User-Agent", "opengpu-cli-updater");
    if let Ok(token) = env::var("OPENGPU_GITHUB_TOKEN") {
        if !token.trim().is_empty() {
            request = request.set("Authorization", &format!("Bearer {}", token.trim()));
        }
    }
    let response = request
        .call()
        .map_err(|error| format!("download failed for {url}: {error}"))?;
    let mut bytes = Vec::new();
    response
        .into_reader()
        .read_to_end(&mut bytes)
        .map_err(|error| format!("failed reading {url}: {error}"))?;
    Ok(bytes)
}

fn verify_checksum(asset_name: &str, bytes: &[u8], checksum_file: &str) -> Result<(), String> {
    let expected = checksum_file
        .split_whitespace()
        .next()
        .filter(|value| value.len() == 64 && value.chars().all(|ch| ch.is_ascii_hexdigit()))
        .ok_or_else(|| format!("invalid SHA-256 file for {asset_name}"))?;
    let actual = hex::encode(Sha256::digest(bytes));
    if !actual.eq_ignore_ascii_case(expected) {
        return Err(format!(
            "SHA-256 mismatch for {asset_name}: expected {expected}, got {actual}"
        ));
    }
    Ok(())
}

fn smoke_check(path: &Path, label: &str) -> Result<(), String> {
    let status = Command::new(path)
        .arg("--version")
        .status()
        .map_err(|error| format!("cannot run staged {label}: {error}"))?;
    if !status.success() {
        return Err(format!("staged {label} failed its --version smoke check"));
    }
    Ok(())
}

fn replace_binary(staged: &Path, target: &Path) -> Result<(), String> {
    fs::rename(staged, target).map_err(|error| {
        format!(
            "cannot replace {} with verified update: {error}",
            target.display()
        )
    })
}

#[cfg(unix)]
fn set_executable(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))
        .map_err(|error| format!("cannot mark {} executable: {error}", path.display()))
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{parse_linux_release_assets, release_target, verify_checksum};
    use sha2::{Digest, Sha256};

    #[test]
    fn selects_supported_linux_release_targets() {
        assert_eq!(
            release_target("linux", "aarch64").as_deref(),
            Ok("aarch64-unknown-linux-gnu")
        );
        assert_eq!(
            release_target("linux", "x86_64").as_deref(),
            Ok("x86_64-unknown-linux-gnu")
        );
        assert!(release_target("linux", "riscv64").is_err());
        assert!(release_target("macos", "aarch64").is_err());
    }

    #[test]
    fn accepts_release_checksum_file_format() {
        let bytes = b"verified release";
        let digest = hex::encode(Sha256::digest(bytes));
        let checksum = format!("{digest}  opengpu-aarch64-unknown-linux-gnu\n");
        assert!(verify_checksum("opengpu-aarch64-unknown-linux-gnu", bytes, &checksum).is_ok());
    }

    #[test]
    fn rejects_mismatched_and_malformed_checksums() {
        assert!(verify_checksum(
            "opengpu",
            b"release",
            &format!("{}  opengpu", "0".repeat(64))
        )
        .is_err());
        assert!(verify_checksum("opengpu", b"release", "not-a-checksum").is_err());
    }

    #[test]
    fn selects_linux_release_and_exact_architecture_assets() {
        let releases = serde_json::json!([
            {
                "tag_name": "cli-windows-v9.0.0",
                "draft": false,
                "prerelease": false,
                "assets": []
            },
            {
                "tag_name": "cli-linux-v0.1.18",
                "draft": false,
                "prerelease": false,
                "assets": [
                    {
                        "name": "opengpu-aarch64-unknown-linux-gnu",
                        "browser_download_url": "https://example.test/opengpu"
                    },
                    {
                        "name": "opengpu-aarch64-unknown-linux-gnu.sha256",
                        "browser_download_url": "https://example.test/opengpu.sha256"
                    },
                    {
                        "name": "opengpu-node-agent-aarch64-unknown-linux-gnu",
                        "browser_download_url": "https://example.test/agent"
                    },
                    {
                        "name": "opengpu-node-agent-aarch64-unknown-linux-gnu.sha256",
                        "browser_download_url": "https://example.test/agent.sha256"
                    }
                ]
            }
        ]);
        let assets = parse_linux_release_assets(
            &releases.to_string(),
            "opengpu-aarch64-unknown-linux-gnu",
            "opengpu-node-agent-aarch64-unknown-linux-gnu",
        )
        .expect("Linux assets should resolve");

        assert_eq!(assets.tag, "cli-linux-v0.1.18");
        assert_eq!(assets.cli_url, "https://example.test/opengpu");
        assert_eq!(assets.agent_url, "https://example.test/agent");
    }
}
