use crate::contracts::{CodeVerificationCheck, CodeVerificationEvidence};
use serde_json::Value;
use std::env;
use std::ffi::OsString;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant};

const DEFAULT_JAVA_IMAGE: &str = "docker.io/library/eclipse-temurin:21-jdk";
const COMPILE_TIMEOUT: Duration = Duration::from_secs(30);
const RUN_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_DIAGNOSTIC_CHARS: usize = 4_096;

#[derive(Clone, Debug)]
struct PodmanVerifierConfig {
    binary: OsString,
    image: String,
}

#[derive(Debug)]
struct ContainerResult {
    success: bool,
    timed_out: bool,
    stdout: String,
}

static PODMAN_VERIFIER: OnceLock<Option<PodmanVerifierConfig>> = OnceLock::new();

pub fn podman_java_verifier_available() -> bool {
    podman_verifier_config().is_some()
}

pub fn verify_generated_java(
    prompt: &str,
    worker_output: &str,
) -> Option<CodeVerificationEvidence> {
    let config = podman_verifier_config()?;
    let source = extract_java_source(worker_output)?;
    let class_name = public_java_class_name(&source)?;
    let started = Instant::now();
    let workspace = VerificationWorkspace::create().ok()?;
    let source_path = workspace.path.join(format!("{class_name}.java"));
    if fs::write(&source_path, source.as_bytes()).is_err() {
        return None;
    }

    let compile = run_container(
        config,
        &workspace.path,
        "javac",
        &["javac", "-proc:none", &format!("{class_name}.java")],
        COMPILE_TIMEOUT,
    );
    let compiler_passed = compile.as_ref().is_ok_and(|result| result.success);
    let mut checks = vec![CodeVerificationCheck {
        check_id: "java_compile".to_string(),
        passed: compiler_passed,
        detail: if compiler_passed {
            "`javac -proc:none` completed successfully in the isolated contributor sandbox."
                .to_string()
        } else {
            "Java compilation failed or exceeded its sandbox timeout.".to_string()
        },
    }];

    let runtime = if compiler_passed {
        run_container(
            config,
            &workspace.path,
            "java",
            &["java", "-cp", "/workspace", &class_name],
            RUN_TIMEOUT,
        )
        .ok()
    } else {
        None
    };
    let runtime_passed = runtime.as_ref().is_some_and(|result| result.success);
    checks.push(CodeVerificationCheck {
        check_id: "java_runtime".to_string(),
        passed: runtime_passed,
        detail: if runtime_passed {
            "The generated Java entrypoint exited successfully with network disabled.".to_string()
        } else {
            "The generated Java entrypoint failed or exceeded its five-second sandbox timeout."
                .to_string()
        },
    });

    let expected = exact_fibonacci_output_contract(prompt);
    let semantic_passed = expected.as_ref().is_some_and(|expected| {
        runtime
            .as_ref()
            .map(|result| normalized_output_lines(&result.stdout) == *expected)
            .unwrap_or(false)
    });
    checks.push(CodeVerificationCheck {
        check_id: "runtime_output_contract".to_string(),
        passed: semantic_passed,
        detail: if let Some(expected) = expected.as_ref() {
            if semantic_passed {
                format!(
                    "Runtime output matched all {} exact required lines.",
                    expected.len()
                )
            } else {
                format!(
                    "Runtime output did not exactly match the {} required `n=value` lines.",
                    expected.len()
                )
            }
        } else {
            "No deterministic runtime oracle is implemented for this request type.".to_string()
        },
    });

    let execution_verified = compiler_passed && runtime_passed && semantic_passed;
    let status = if !compiler_passed || !runtime_passed || (expected.is_some() && !semantic_passed)
    {
        "failed"
    } else if execution_verified {
        "passed"
    } else {
        "unverified"
    };
    let diagnostics = combined_diagnostics(compile.as_ref().ok(), runtime.as_ref());

    Some(CodeVerificationEvidence {
        verifier: "rootless-podman-java-v1".to_string(),
        language: "java".to_string(),
        status: status.to_string(),
        execution_verified,
        compiler_passed,
        runtime_passed,
        semantic_passed,
        duration_ms: started.elapsed().as_millis() as u64,
        checks,
        diagnostics,
    })
}

fn podman_verifier_config() -> Option<&'static PodmanVerifierConfig> {
    PODMAN_VERIFIER.get_or_init(detect_podman_verifier).as_ref()
}

fn detect_podman_verifier() -> Option<PodmanVerifierConfig> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    if !env::var("OPENGPU_CODE_VERIFIER")
        .map(|value| value.eq_ignore_ascii_case("podman"))
        .unwrap_or(false)
    {
        return None;
    }
    let binary = env::var_os("OPENGPU_PODMAN_BIN").unwrap_or_else(|| "podman".into());
    let info = Command::new(&binary)
        .args(["info", "--format", "json"])
        .stdin(Stdio::null())
        .output()
        .ok()?;
    if !info.status.success() || !podman_info_is_rootless(&info.stdout) {
        return None;
    }
    let image =
        env::var("OPENGPU_JAVA_VERIFIER_IMAGE").unwrap_or_else(|_| DEFAULT_JAVA_IMAGE.to_string());
    let image_exists = Command::new(&binary)
        .args(["image", "exists", &image])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()?
        .success();
    if !image_exists {
        return None;
    }
    let image_id = Command::new(&binary)
        .args(["image", "inspect", "--format", "{{.Id}}", &image])
        .stdin(Stdio::null())
        .output()
        .ok()?;
    let image_id = String::from_utf8(image_id.stdout).ok()?.trim().to_string();
    if !image_id.starts_with("sha256:") {
        return None;
    }
    Some(PodmanVerifierConfig {
        binary,
        image: image_id,
    })
}

fn podman_info_is_rootless(raw: &[u8]) -> bool {
    serde_json::from_slice::<Value>(raw)
        .ok()
        .and_then(|value| {
            value
                .pointer("/host/security/rootless")
                .or_else(|| value.pointer("/Host/Security/Rootless"))
                .and_then(Value::as_bool)
        })
        .unwrap_or(false)
}

fn run_container(
    config: &PodmanVerifierConfig,
    workspace: &Path,
    stage: &str,
    container_command: &[&str],
    timeout: Duration,
) -> Result<ContainerResult, String> {
    let container_name = format!("mundusx-verify-{stage}-{}", uuid::Uuid::new_v4().simple());
    let stdout_path = workspace.join(format!("{stage}.stdout"));
    let stderr_path = workspace.join(format!("{stage}.stderr"));
    let stdout = File::create(&stdout_path)
        .map_err(|error| format!("failed to create verifier stdout: {error}"))?;
    let stderr = File::create(&stderr_path)
        .map_err(|error| format!("failed to create verifier stderr: {error}"))?;
    let mount = format!("type=bind,src={},dst=/workspace,rw", workspace.display());
    let mut args = vec![
        OsString::from("run"),
        OsString::from("--rm"),
        OsString::from("--pull=never"),
        OsString::from("--network=none"),
        OsString::from("--read-only"),
        OsString::from("--cap-drop=ALL"),
        OsString::from("--security-opt=no-new-privileges"),
        OsString::from("--pids-limit=64"),
        OsString::from("--memory=512m"),
        OsString::from("--memory-swap=512m"),
        OsString::from("--cpus=1"),
        OsString::from("--userns=keep-id"),
        OsString::from("--tmpfs=/tmp:rw,noexec,nosuid,nodev,size=64m"),
        OsString::from("--env=HOME=/tmp"),
        OsString::from("--mount"),
        OsString::from(mount),
        OsString::from("--workdir=/workspace"),
        OsString::from("--name"),
        OsString::from(&container_name),
        OsString::from(&config.image),
    ];
    args.extend(container_command.iter().map(OsString::from));
    let mut child = Command::new(&config.binary)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()
        .map_err(|error| format!("failed to launch rootless Podman verifier: {error}"))?;
    let deadline = Instant::now() + timeout;
    let (success, timed_out) = loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("failed to poll Podman verifier: {error}"))?
        {
            break (status.success(), false);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let _ = Command::new(&config.binary)
                .args(["rm", "--force", &container_name])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            break (false, true);
        }
        thread::sleep(Duration::from_millis(50));
    };
    Ok(ContainerResult {
        success,
        timed_out,
        stdout: read_diagnostic(&stdout_path),
    })
}

fn extract_java_source(worker_output: &str) -> Option<String> {
    let output = worker_output
        .split_once("response=")
        .map(|(_, response)| response)
        .unwrap_or(worker_output);
    let mut in_java = false;
    let mut lines = Vec::new();
    for line in output.lines() {
        if !in_java {
            if line
                .trim_start()
                .to_ascii_lowercase()
                .starts_with("```java")
            {
                in_java = true;
            }
            continue;
        }
        if line.trim_start().starts_with("```") {
            break;
        }
        lines.push(line);
    }
    (!lines.is_empty()).then(|| lines.join("\n"))
}

fn public_java_class_name(source: &str) -> Option<String> {
    let marker = "public class ";
    let start = source.find(marker)? + marker.len();
    let name = source[start..]
        .chars()
        .take_while(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '$'))
        .collect::<String>();
    (!name.is_empty()).then_some(name)
}

fn exact_fibonacci_output_contract(prompt: &str) -> Option<Vec<String>> {
    let prompt = prompt.to_ascii_lowercase();
    if !(prompt.contains("fibonacci")
        && prompt.contains("exactly 21 lines")
        && prompt.contains("format n=value")
        && prompt.contains("0 through 20"))
    {
        return None;
    }
    let mut previous = 0u64;
    let mut current = 1u64;
    let mut expected = Vec::new();
    for n in 0..=20 {
        let value = if n == 0 { 0 } else { current };
        expected.push(format!("{n}={value}"));
        if n > 0 {
            let next = previous + current;
            previous = current;
            current = next;
        }
    }
    Some(expected)
}

fn normalized_output_lines(output: &str) -> Vec<String> {
    output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

fn combined_diagnostics(
    compile: Option<&ContainerResult>,
    runtime: Option<&ContainerResult>,
) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(compile) = compile {
        if compile.timed_out {
            parts.push("compile timed out".to_string());
        }
        if !compile.success && !compile.timed_out {
            parts.push("compile exited unsuccessfully".to_string());
        }
    }
    if let Some(runtime) = runtime {
        if runtime.timed_out {
            parts.push("runtime timed out".to_string());
        }
        if !runtime.success && !runtime.timed_out {
            parts.push("runtime exited unsuccessfully".to_string());
        }
    }
    (!parts.is_empty()).then(|| truncate_and_sanitize(&parts.join("; ")))
}

fn read_diagnostic(path: &Path) -> String {
    fs::read_to_string(path)
        .map(|value| truncate_and_sanitize(&value))
        .unwrap_or_default()
}

fn truncate_and_sanitize(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_control() || matches!(character, '\n' | '\r' | '\t'))
        .take(MAX_DIAGNOSTIC_CHARS)
        .collect()
}

struct VerificationWorkspace {
    path: PathBuf,
}

impl VerificationWorkspace {
    fn create() -> Result<Self, String> {
        let path = env::temp_dir().join(format!(
            "mundusx-code-verify-{}",
            uuid::Uuid::new_v4().simple()
        ));
        fs::create_dir(&path)
            .map_err(|error| format!("failed to create verifier workspace: {error}"))?;
        Ok(Self { path })
    }
}

impl Drop for VerificationWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_java_without_worker_transport_metadata() {
        let source = extract_java_source(
            "contributed-cluster mode=test; response=```java\npublic class Demo {}\n```",
        )
        .expect("java source");
        assert_eq!(source, "public class Demo {}");
        assert_eq!(public_java_class_name(&source).as_deref(), Some("Demo"));
    }

    #[test]
    fn builds_exact_fibonacci_runtime_oracle() {
        let output = exact_fibonacci_output_contract(
            "Print exactly 21 lines in the format n=value for Fibonacci 0 through 20.",
        )
        .expect("oracle");
        assert_eq!(output.len(), 21);
        assert_eq!(output[0], "0=0");
        assert_eq!(output[10], "10=55");
        assert_eq!(output[20], "20=6765");
    }

    #[test]
    fn recognizes_only_rootless_podman_info() {
        assert!(podman_info_is_rootless(
            br#"{"host":{"security":{"rootless":true}}}"#
        ));
        assert!(!podman_info_is_rootless(
            br#"{"host":{"security":{"rootless":false}}}"#
        ));
    }
}
