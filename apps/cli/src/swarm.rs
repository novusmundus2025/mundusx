use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use uuid::Uuid;

const MAX_WORKERS: usize = 8;

pub struct TestOptions {
    pub workspace: PathBuf,
    pub tasks: Vec<String>,
    pub workers: usize,
    pub fail_fast: bool,
    pub fallback: Option<String>,
    pub json: bool,
}

#[derive(Clone, Debug, Deserialize)]
struct ManifestTask {
    name: String,
    command: String,
}

#[derive(Debug, Deserialize)]
struct TestManifest {
    #[serde(default)]
    enabled: bool,
    #[serde(default = "default_workers")]
    workers: usize,
    #[serde(default)]
    fail_fast: bool,
    #[serde(default)]
    fallback: Option<String>,
    #[serde(default)]
    tasks: Vec<ManifestTask>,
}

#[derive(Debug, Deserialize)]
struct SwarmManifest {
    #[serde(default = "manifest_version")]
    version: u32,
    test: TestManifest,
}

#[derive(Clone, Debug)]
struct TestTask {
    name: String,
    command: String,
}

#[derive(Debug, Serialize)]
struct TestResult {
    name: String,
    command: String,
    success: bool,
    exit_code: Option<i32>,
    duration_ms: u128,
    log: String,
    error: Option<String>,
}

fn default_workers() -> usize {
    2
}

fn manifest_version() -> u32 {
    1
}

fn parse_task(value: &str) -> Result<TestTask, String> {
    let (name, command) = value
        .split_once('=')
        .ok_or_else(|| format!("invalid task `{value}`; expected NAME=COMMAND"))?;
    let name = name.trim();
    let command = command.trim();
    if name.is_empty() || command.is_empty() {
        return Err(format!(
            "invalid task `{value}`; name and command are required"
        ));
    }
    if name.contains('/') || name.contains('\\') || name == "." || name == ".." {
        return Err(format!("invalid task name `{name}`"));
    }
    Ok(TestTask {
        name: name.to_string(),
        command: command.to_string(),
    })
}

fn read_manifest(workspace: &Path) -> Result<Option<SwarmManifest>, String> {
    let path = workspace.join(".mundusx").join("swarm.json");
    if !path.is_file() {
        return Ok(None);
    }
    let bytes =
        fs::read(&path).map_err(|error| format!("could not read {}: {error}", path.display()))?;
    let manifest: SwarmManifest = serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid {}: {error}", path.display()))?;
    if manifest.version != 1 {
        return Err(format!(
            "unsupported swarm manifest version {}; expected 1",
            manifest.version
        ));
    }
    Ok(Some(manifest))
}

fn safe_name(name: &str) -> String {
    let value: String = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '-'
            }
        })
        .collect();
    value.trim_matches('-').chars().take(80).collect::<String>()
}

fn shell_command(command: &str) -> Command {
    if cfg!(windows) {
        let mut child = Command::new("cmd.exe");
        child.args(["/D", "/C", command]);
        child
    } else {
        let mut child = Command::new("sh");
        child.args(["-lc", command]);
        child
    }
}

fn run_one(task: TestTask, workspace: &Path, run_dir: &Path, lane: usize) -> TestResult {
    let started = Instant::now();
    let task_name = safe_name(&task.name);
    let log_path = run_dir.join(format!("{:02}-{}.log", lane + 1, task_name));
    let temp_path = run_dir.join(format!("{:02}-{}-tmp", lane + 1, task_name));
    let _ = fs::create_dir_all(&temp_path);
    let output = shell_command(&task.command)
        .current_dir(workspace)
        .env("MUNDUSX_SWARM_LANE", lane.to_string())
        .env("MUNDUSX_SWARM_TASK", &task.name)
        .env("TMP", &temp_path)
        .env("TEMP", &temp_path)
        .env("TMPDIR", &temp_path)
        .output();
    match output {
        Ok(output) => {
            let mut log = Vec::new();
            log.extend_from_slice(&output.stdout);
            if !output.stdout.is_empty() && !output.stderr.is_empty() {
                log.extend_from_slice(b"\n--- stderr ---\n");
            }
            log.extend_from_slice(&output.stderr);
            let write_error = fs::write(&log_path, log)
                .err()
                .map(|error| error.to_string());
            TestResult {
                name: task.name,
                command: task.command,
                success: output.status.success() && write_error.is_none(),
                exit_code: output.status.code(),
                duration_ms: started.elapsed().as_millis(),
                log: log_path.display().to_string(),
                error: write_error,
            }
        }
        Err(error) => TestResult {
            name: task.name,
            command: task.command,
            success: false,
            exit_code: None,
            duration_ms: started.elapsed().as_millis(),
            log: log_path.display().to_string(),
            error: Some(format!("could not start test lane: {error}")),
        },
    }
}

fn run_sequential_fallback(command: &str, workspace: &Path, run_dir: &Path) -> TestResult {
    run_one(
        TestTask {
            name: "sequential-fallback".to_string(),
            command: command.to_string(),
        },
        workspace,
        run_dir,
        0,
    )
}

pub fn run_tests(options: TestOptions) -> Result<(), String> {
    if !(1..=MAX_WORKERS).contains(&options.workers) {
        return Err(format!("--workers must be between 1 and {MAX_WORKERS}"));
    }
    let workspace = fs::canonicalize(&options.workspace).map_err(|error| {
        format!(
            "could not open project workspace {}: {error}",
            options.workspace.display()
        )
    })?;
    if !workspace.is_dir() {
        return Err(format!(
            "workspace is not a directory: {}",
            workspace.display()
        ));
    }

    let manifest = if options.tasks.is_empty() {
        read_manifest(&workspace)?
    } else {
        None
    };
    let explicitly_enabled = !options.tasks.is_empty();
    let mut workers = options.workers.clamp(1, MAX_WORKERS);
    let mut fail_fast = options.fail_fast;
    let mut fallback = options.fallback;
    let tasks = if explicitly_enabled {
        options
            .tasks
            .iter()
            .map(|value| parse_task(value))
            .collect::<Result<Vec<_>, _>>()?
    } else if let Some(manifest) = manifest {
        if !manifest.test.enabled {
            if let Some(command) = fallback.or(manifest.test.fallback) {
                let run_dir = prepare_run_dir(&workspace)?;
                let result = run_sequential_fallback(&command, &workspace, &run_dir);
                return finish(vec![result], &run_dir, options.json, true);
            }
            return Err("project swarm is disabled in .mundusx/swarm.json".to_string());
        }
        workers = manifest.test.workers.clamp(1, MAX_WORKERS);
        fail_fast |= manifest.test.fail_fast;
        fallback = fallback.or(manifest.test.fallback);
        manifest
            .test
            .tasks
            .into_iter()
            .map(|task| parse_task(&format!("{}={}", task.name, task.command)))
            .collect::<Result<Vec<_>, _>>()?
    } else {
        return Err(
            "no test lanes supplied; pass --task NAME=COMMAND or add .mundusx/swarm.json"
                .to_string(),
        );
    };

    let run_dir = prepare_run_dir(&workspace)?;
    if tasks.len() < 2 || workers == 1 {
        let command = fallback.or_else(|| tasks.first().map(|task| task.command.clone()));
        if let Some(command) = command {
            let result = run_sequential_fallback(&command, &workspace, &run_dir);
            return finish(vec![result], &run_dir, options.json, true);
        }
        return Err("parallel test execution requires at least two independent lanes".to_string());
    }

    let queue = Arc::new(Mutex::new(VecDeque::from(tasks)));
    let results = Arc::new(Mutex::new(Vec::new()));
    let failed = Arc::new(AtomicBool::new(false));
    let worker_count = workers.min(queue.lock().unwrap().len());
    let mut handles = Vec::with_capacity(worker_count);
    for lane in 0..worker_count {
        let queue = Arc::clone(&queue);
        let results = Arc::clone(&results);
        let failed = Arc::clone(&failed);
        let workspace = workspace.clone();
        let run_dir = run_dir.clone();
        handles.push(thread::spawn(move || loop {
            if fail_fast && failed.load(Ordering::Acquire) {
                break;
            }
            let task = queue.lock().unwrap().pop_front();
            let Some(task) = task else { break };
            let result = run_one(task, &workspace, &run_dir, lane);
            if !result.success {
                failed.store(true, Ordering::Release);
            }
            results.lock().unwrap().push(result);
        }));
    }

    let mut join_failed = false;
    for handle in handles {
        if handle.join().is_err() {
            join_failed = true;
        }
    }
    let mut results = Arc::try_unwrap(results)
        .map_err(|_| "could not collect swarm results".to_string())?
        .into_inner()
        .map_err(|_| "could not collect swarm results".to_string())?;
    results.sort_by(|left, right| left.name.cmp(&right.name));
    if join_failed {
        if let Some(command) = fallback {
            results.push(run_sequential_fallback(&command, &workspace, &run_dir));
        } else {
            return Err(format!(
                "a swarm worker stopped unexpectedly; logs: {}",
                run_dir.display()
            ));
        }
    }
    finish(results, &run_dir, options.json, false)
}

fn prepare_run_dir(workspace: &Path) -> Result<PathBuf, String> {
    let path = workspace
        .join(".hermes")
        .join("swarm")
        .join(Uuid::new_v4().to_string());
    fs::create_dir_all(&path)
        .map_err(|error| format!("could not create swarm log directory: {error}"))?;
    Ok(path)
}

fn finish(
    results: Vec<TestResult>,
    run_dir: &Path,
    json_output: bool,
    fallback_used: bool,
) -> Result<(), String> {
    let success = !results.is_empty() && results.iter().all(|result| result.success);
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "success": success,
                "fallback_used": fallback_used,
                "run_directory": run_dir,
                "results": results,
            }))
            .map_err(|error| error.to_string())?
        );
    } else {
        println!(
            "Project Swarm: {} ({} lane{})",
            if success { "passed" } else { "failed" },
            results.len(),
            if results.len() == 1 { "" } else { "s" }
        );
        for result in &results {
            println!(
                "  {} {:<24} {:>7.2}s  {}",
                if result.success { "PASS" } else { "FAIL" },
                result.name,
                Duration::from_millis(result.duration_ms as u64).as_secs_f64(),
                result.log
            );
            if let Some(error) = &result.error {
                println!("       {error}");
            }
        }
        if fallback_used {
            println!("  Parallel plan unavailable; used the sequential fallback.");
        }
    }
    if success {
        Ok(())
    } else {
        Err(format!(
            "one or more test lanes failed; logs: {}",
            run_dir.display()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_named_task_without_losing_equals_in_command() {
        let task = parse_task("unit=python -c \"assert 1 == 1\"").unwrap();
        assert_eq!(task.name, "unit");
        assert_eq!(task.command, "python -c \"assert 1 == 1\"");
    }

    #[test]
    fn rejects_path_like_task_names() {
        assert!(parse_task("../unit=echo ok").is_err());
        assert!(parse_task("folder/unit=echo ok").is_err());
    }

    #[test]
    fn safe_name_normalizes_log_file_component() {
        assert_eq!(safe_name("api tests (fast)"), "api-tests--fast");
    }
}
