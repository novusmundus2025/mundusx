use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use uuid::Uuid;

const MAX_TASKS: usize = 4;
const DEFAULT_WORKERS: usize = 2;

#[derive(Clone)]
pub struct CoordinatorOptions {
    pub workspace: PathBuf,
    pub objective: String,
    pub session_id: String,
    pub task_id: String,
    pub connection_id: String,
    pub remote_base: String,
    pub token: String,
    pub data_dir: PathBuf,
    pub cancellation: Arc<AtomicBool>,
    pub event_sender: mpsc::Sender<Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct CodingTask {
    id: String,
    objective: String,
    #[serde(default)]
    depends_on: Vec<String>,
    owned_paths: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct CodingPlan {
    summary: String,
    tasks: Vec<CodingTask>,
    verification_command: String,
}

#[derive(Clone, Debug, Serialize)]
struct JournalEvent {
    state: String,
    task: Option<String>,
    detail: String,
}

struct WorktreeSet {
    repository: PathBuf,
    paths: Vec<PathBuf>,
}

impl WorktreeSet {
    fn new(repository: PathBuf) -> Self {
        Self {
            repository,
            paths: Vec::new(),
        }
    }

    fn add(&mut self, path: PathBuf) {
        self.paths.push(path);
    }
}

impl Drop for WorktreeSet {
    fn drop(&mut self) {
        for path in self.paths.iter().rev() {
            let _ = Command::new("git")
                .args(["worktree", "remove", "--force"])
                .arg(path)
                .current_dir(&self.repository)
                .output();
        }
    }
}

pub fn requested(prompt: &str) -> bool {
    let value = prompt.to_ascii_lowercase();
    [
        "project swarm",
        "parallel coding",
        "parallel code",
        "code in parallel",
        "work in parallel",
        "separate functions at the same time",
    ]
    .iter()
    .any(|marker| value.contains(marker))
}

pub fn should_coordinate(prompt: &str) -> bool {
    let value = prompt.to_ascii_lowercase();
    if [
        "do not use project swarm",
        "don't use project swarm",
        "without project swarm",
        "use one worker",
        "single worker",
        "work sequentially",
    ]
    .iter()
    .any(|marker| value.contains(marker))
    {
        return false;
    }
    if requested(prompt) {
        return true;
    }

    let substantial_scope = [
        "complete api",
        "entire api",
        "crud api",
        "full api",
        "build an api",
        "create an api",
        "build a frontend",
        "create a frontend",
        "build a backend",
        "create a backend",
        "build an app",
        "create an app",
        "build an application",
        "create an application",
        "build a website",
        "create a website",
        "build a dashboard",
        "create a dashboard",
        "full stack",
        "full-stack",
        "end-to-end",
        "end to end",
        "from scratch",
        "multiple modules",
        "several modules",
    ]
    .iter()
    .any(|marker| value.contains(marker));
    if substantial_scope {
        return true;
    }

    let has_build_action = [
        "add ",
        "build ",
        "create ",
        "develop ",
        "implement ",
        "migrate ",
        "refactor ",
        "rewrite ",
    ]
    .iter()
    .any(|marker| value.contains(marker));
    let domain_count = [
        " api",
        "frontend",
        "front end",
        "backend",
        "back end",
        "database",
        "authentication",
        " test",
        "documentation",
    ]
    .iter()
    .filter(|marker| value.contains(**marker))
    .count();
    has_build_action && domain_count >= 2
}

pub fn run(options: CoordinatorOptions) -> Result<Value, String> {
    require_clean_git_repository(&options.workspace)?;
    let run_id = Uuid::new_v4().simple().to_string();
    let run_root = std::env::temp_dir().join(format!("mundusx-project-swarm-{run_id}"));
    fs::create_dir_all(&run_root)
        .map_err(|error| format!("could not create Project Swarm workspace: {error}"))?;
    let journal_dir = options.data_dir.join("project-swarm").join(&run_id);
    fs::create_dir_all(&journal_dir)
        .map_err(|error| format!("could not create Project Swarm journal: {error}"))?;
    let journal_path = journal_dir.join("journal.json");
    let mut journal = Vec::new();
    record(
        &journal_path,
        &mut journal,
        "planning",
        None,
        "Planner is analyzing independent project work",
    )?;
    emit(
        &options,
        "swarm_planning",
        "Project Swarm is planning independent work",
        json!({"workers": DEFAULT_WORKERS}),
    );

    let planner_prompt = planner_prompt(&options.objective);
    let planner_data = journal_dir.join("planner");
    let planner = run_hermes(
        &options,
        &planner_prompt,
        &format!("{}-swarm-planner", options.session_id),
        &options.workspace,
        &planner_data,
        false,
    )?;
    let content = planner
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .ok_or("Project Swarm planner returned no plan")?;
    let plan: CodingPlan = serde_json::from_str(extract_json(content)?)
        .map_err(|error| format!("Project Swarm planner returned invalid JSON: {error}"))?;
    validate_plan(&plan)?;
    fs::write(
        journal_dir.join("plan.json"),
        serde_json::to_vec_pretty(&plan).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("could not save Project Swarm plan: {error}"))?;
    record(&journal_path, &mut journal, "planned", None, &plan.summary)?;
    emit(
        &options,
        "swarm_planned",
        "Project Swarm plan is ready",
        json!({"tasks": plan.tasks.len(), "summary": plan.summary}),
    );

    let base = git_output(&options.workspace, &["rev-parse", "HEAD"])?;
    let integration = run_root.join("integration");
    git_ok(
        &options.workspace,
        &[
            "worktree",
            "add",
            "--detach",
            path_text(&integration)?,
            base.trim(),
        ],
    )?;
    let mut worktrees = WorktreeSet::new(options.workspace.clone());
    worktrees.add(integration.clone());
    let mut completed = BTreeSet::new();
    let mut pending = plan
        .tasks
        .iter()
        .cloned()
        .map(|task| (task.id.clone(), task))
        .collect::<BTreeMap<_, _>>();
    let mut worker_summaries = Vec::new();

    while !pending.is_empty() {
        if options.cancellation.load(Ordering::Acquire) {
            return Err("Project Swarm was cancelled".to_string());
        }
        let ready = pending
            .values()
            .filter(|task| task.depends_on.iter().all(|id| completed.contains(id)))
            .take(DEFAULT_WORKERS)
            .cloned()
            .collect::<Vec<_>>();
        if ready.is_empty() {
            return Err("Project Swarm plan cannot make dependency progress".to_string());
        }
        let integration_head = git_output(&integration, &["rev-parse", "HEAD"])?;
        let mut prepared = Vec::new();
        for task in ready {
            let worker_path = run_root.join(format!("worker-{}", task.id));
            git_ok(
                &options.workspace,
                &[
                    "worktree",
                    "add",
                    "--detach",
                    path_text(&worker_path)?,
                    integration_head.trim(),
                ],
            )?;
            worktrees.add(worker_path.clone());
            prepared.push((task, worker_path));
        }

        let mut handles = Vec::new();
        for (task, worker_path) in prepared {
            let worker_options = options.clone();
            let worker_journal = journal_dir.clone();
            emit(
                &options,
                "swarm_worker_started",
                &format!("{} started", task.id),
                json!({"worker": task.id, "owned_paths": task.owned_paths}),
            );
            handles.push(thread::spawn(move || {
                run_worker(&worker_options, &task, &worker_path, &worker_journal)
                    .map(|(commit, summary)| (task, worker_path, commit, summary))
            }));
        }

        let mut wave = Vec::new();
        for handle in handles {
            wave.push(
                handle
                    .join()
                    .map_err(|_| "Project Swarm worker stopped unexpectedly".to_string())??,
            );
        }
        wave.sort_by(|left, right| left.0.id.cmp(&right.0.id));
        for (task, _worker_path, commit, summary) in wave {
            git_ok(&integration, &["cherry-pick", &commit])?;
            record(
                &journal_path,
                &mut journal,
                "integrated",
                Some(&task.id),
                &commit,
            )?;
            emit(
                &options,
                "swarm_worker_completed",
                &format!("{} completed", task.id),
                json!({"worker": task.id, "commit": commit}),
            );
            completed.insert(task.id.clone());
            pending.remove(&task.id);
            worker_summaries.push(format!("{}: {}", task.id, summary.trim()));
        }
    }

    record(
        &journal_path,
        &mut journal,
        "reviewing",
        None,
        "Hermes is reviewing the integrated result",
    )?;
    emit(
        &options,
        "swarm_reviewing",
        "Hermes is reviewing the integrated result",
        json!({}),
    );
    let frontend_marker = if requires_browser_acceptance(&options.objective) {
        "\n\nMUNDUSX_FRONTEND_ACCEPTANCE_V1: Start the integrated application and use browser automation to verify meaningful visible content, browser errors, and layout at desktop and narrow widths. Fix failures and repeat the browser checks. Stop services before completing."
    } else {
        ""
    };
    let review_prompt = format!(
        "Act as the Project Swarm integration reviewer. Inspect the combined implementation for the original request below. Resolve integration defects in this temporary worktree, run the complete relevant tests, and leave the integrated project ready to publish. Do not deploy or publish externally.{frontend_marker}\n\nOriginal request:\n{}",
        options.objective
    );
    let review = run_hermes(
        &options,
        &review_prompt,
        &format!("{}-swarm-review", options.session_id),
        &integration,
        &journal_dir.join("review"),
        true,
    )?;
    if !changed_paths(&integration)?.is_empty() {
        git_ok(&integration, &["add", "--all"])?;
        git_ok(
            &integration,
            &[
                "-c",
                "user.name=MundusX Project Swarm",
                "-c",
                "user.email=swarm@mundusx.invalid",
                "commit",
                "-m",
                "Project Swarm: integrate and verify",
            ],
        )?;
    }

    record(
        &journal_path,
        &mut journal,
        "verifying",
        None,
        &plan.verification_command,
    )?;
    emit(
        &options,
        "swarm_verifying",
        "Project Swarm is verifying the integrated result",
        json!({}),
    );
    let verification = run_shell(&integration, &plan.verification_command)?;
    fs::write(journal_dir.join("verification.log"), &verification.output)
        .map_err(|error| format!("could not save Project Swarm verification: {error}"))?;
    if !verification.success {
        return Err(format!(
            "Project Swarm integration verification failed; original project was not changed. Log: {}",
            journal_dir.join("verification.log").display()
        ));
    }

    let integrated = publish_integration(&options.workspace, base.trim(), &integration)?;
    record(&journal_path, &mut journal, "completed", None, &integrated)?;
    emit(
        &options,
        "swarm_completed",
        "Project Swarm applied the verified integration",
        json!({"commit": integrated, "tasks": completed.len()}),
    );

    let mut review_events = review["events"].as_array().cloned().unwrap_or_default();
    review_events.push(json!({"type":"tool_completed", "data":{"name":"project_swarm", "activity":"test", "verification":true, "success":true}}));
    let mut review_tools = review["tool_calls"].as_array().cloned().unwrap_or_default();
    review_tools.push(json!({"name":"project_swarm", "id":run_id}));
    let mut review_skills = review["skills"].as_array().cloned().unwrap_or_default();
    if !review_skills
        .iter()
        .any(|value| value.as_str() == Some("project-swarm"))
    {
        review_skills.push(json!("project-swarm"));
    }
    Ok(json!({
        "choices": [{"message": {"role": "assistant", "content": format!(
            "Project Swarm completed {} coordinated coding tasks and applied verified commit {}.\n\n{}",
            completed.len(), integrated, worker_summaries.join("\n")
        )}}],
        "runtime": "hermes",
        "events": review_events,
        "tool_calls": review_tools,
        "skills": review_skills,
        "swarm": {"tasks": completed.len(), "commit": integrated, "journal": journal_dir}
    }))
}

fn run_worker(
    options: &CoordinatorOptions,
    task: &CodingTask,
    workspace: &Path,
    journal_dir: &Path,
) -> Result<(String, String), String> {
    let prompt = format!(
        "You are one Project Swarm implementation worker. Work only on this assignment:\n{}\n\nYou exclusively own these project paths: {}. Do not modify any other path, dependency lockfile, root configuration, database migration, or shared generated file. Inspect the project, implement the assignment completely, and run focused validation. Do not commit; the coordinator owns Git integration.",
        task.objective,
        task.owned_paths.join(", ")
    );
    let data_dir = journal_dir.join("workers").join(&task.id);
    let response = run_hermes(
        options,
        &prompt,
        &format!("{}-swarm-{}", options.session_id, task.id),
        workspace,
        &data_dir,
        true,
    )?;
    let changed = changed_paths(workspace)?;
    if changed.is_empty() {
        return Err(format!(
            "Project Swarm worker {} made no project changes",
            task.id
        ));
    }
    for path in &changed {
        if !task
            .owned_paths
            .iter()
            .any(|owned| path_is_owned(path, owned))
        {
            return Err(format!(
                "Project Swarm worker {} changed unowned path `{path}`; original project was not changed",
                task.id
            ));
        }
    }
    git_ok(workspace, &["add", "--all"])?;
    let message = format!("Project Swarm: {}", task.id);
    git_ok(
        workspace,
        &[
            "-c",
            "user.name=MundusX Project Swarm",
            "-c",
            "user.email=swarm@mundusx.invalid",
            "commit",
            "-m",
            &message,
        ],
    )?;
    let commit = git_output(workspace, &["rev-parse", "HEAD"])?;
    let summary = response
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .unwrap_or("Worker completed its assignment.")
        .to_string();
    Ok((commit.trim().to_string(), summary))
}

fn run_hermes(
    options: &CoordinatorOptions,
    prompt: &str,
    session_id: &str,
    workspace: &Path,
    data_dir: &Path,
    approve_mutations: bool,
) -> Result<Value, String> {
    fs::create_dir_all(data_dir)
        .map_err(|error| format!("could not create worker state directory: {error}"))?;
    let sender = options.event_sender.clone();
    let mut callback = move |event: Value| {
        if matches!(
            event["type"].as_str(),
            Some("model_requested" | "model_response_received")
        ) {
            let _ = sender.send(json!({
                "sequence": next_sequence(),
                "event": {
                    "type": event["type"],
                    "summary": if event["type"] == "model_requested" { "Project Swarm worker requested inference" } else { "Project Swarm worker received inference" },
                    "metadata": {"swarm": true}
                }
            }));
        }
    };
    super::hermes_adapter::run(
        prompt,
        session_id,
        workspace,
        data_dir,
        approve_mutations,
        Some(options.cancellation.as_ref()),
        Some((
            &options.remote_base,
            &options.token,
            &options.task_id,
            &options.connection_id,
        )),
        Some(&mut callback),
    )
}

fn planner_prompt(objective: &str) -> String {
    format!(
        "Act as the Project Swarm planner. Inspect this Git project and decompose the authorized request into 2 to {MAX_TASKS} implementation tasks that can safely use isolated worktrees. Return only one JSON object with this exact shape: {{\"summary\":\"...\",\"tasks\":[{{\"id\":\"lowercase-id\",\"objective\":\"...\",\"depends_on\":[],\"owned_paths\":[\"relative/path\"]}}],\"verification_command\":\"one non-interactive command\"}}. Path ownership must be disjoint across every task. Keep dependency lockfiles, root configuration, database migrations, generated schemas, deployment and publishing out of parallel tasks. If shared groundwork is required, make it an earlier task with its own paths. Do not modify files.\n\nAuthorized project request:\n{objective}"
    )
}

fn requires_browser_acceptance(objective: &str) -> bool {
    let value = objective.to_ascii_lowercase();
    [
        "frontend",
        "front-end",
        "website",
        "web app",
        "user interface",
        "react",
        "vue",
        "svelte",
        "angular",
    ]
    .iter()
    .any(|marker| value.contains(marker))
}

fn validate_plan(plan: &CodingPlan) -> Result<(), String> {
    if plan.tasks.len() < 2 || plan.tasks.len() > MAX_TASKS {
        return Err(format!(
            "Project Swarm requires 2 to {MAX_TASKS} safe coding tasks"
        ));
    }
    if plan.verification_command.trim().is_empty() {
        return Err(
            "Project Swarm plan is missing its integration verification command".to_string(),
        );
    }
    if !safe_verification_command(&plan.verification_command) {
        return Err("Project Swarm planner selected an unsafe verification command".to_string());
    }
    let ids = plan
        .tasks
        .iter()
        .map(|task| task.id.as_str())
        .collect::<BTreeSet<_>>();
    if ids.len() != plan.tasks.len() {
        return Err("Project Swarm plan contains duplicate task ids".to_string());
    }
    let mut owners: Vec<(&str, String)> = Vec::new();
    for task in &plan.tasks {
        if !valid_id(&task.id) || task.objective.trim().is_empty() || task.owned_paths.is_empty() {
            return Err(format!("Project Swarm task `{}` is incomplete", task.id));
        }
        if task
            .depends_on
            .iter()
            .any(|dependency| dependency == &task.id || !ids.contains(dependency.as_str()))
        {
            return Err(format!(
                "Project Swarm task `{}` has an invalid dependency",
                task.id
            ));
        }
        for path in &task.owned_paths {
            let normalized = normalize_owned_path(path)?;
            for (owner, existing) in &owners {
                if path_is_owned(&normalized, existing) || path_is_owned(existing, &normalized) {
                    return Err(format!(
                        "Project Swarm tasks `{owner}` and `{}` overlap at `{path}`",
                        task.id
                    ));
                }
            }
            owners.push((task.id.as_str(), normalized));
        }
    }
    let mut completed = BTreeSet::new();
    while completed.len() < plan.tasks.len() {
        let before = completed.len();
        for task in &plan.tasks {
            if task
                .depends_on
                .iter()
                .all(|dependency| completed.contains(dependency))
            {
                completed.insert(task.id.clone());
            }
        }
        if completed.len() == before {
            return Err("Project Swarm plan contains a dependency cycle".to_string());
        }
    }
    Ok(())
}

fn safe_verification_command(command: &str) -> bool {
    let value = command.trim().to_ascii_lowercase();
    if value.is_empty()
        || value
            .chars()
            .any(|character| matches!(character, '&' | '|' | ';' | '>' | '<' | '`'))
    {
        return false;
    }
    [
        "npm test",
        "npm run test",
        "npm run lint",
        "npm run check",
        "npm run build",
        "pnpm test",
        "pnpm run test",
        "pnpm lint",
        "pnpm build",
        "yarn test",
        "yarn lint",
        "yarn build",
        "cargo test",
        "cargo check",
        "cargo build",
        "python -m pytest",
        "python3 -m pytest",
        "pytest",
        "go test",
        "dotnet test",
        "mvn test",
        "gradle test",
        "./gradlew test",
        ".\\gradlew test",
    ]
    .iter()
    .any(|prefix| value == *prefix || value.starts_with(&format!("{prefix} ")))
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('-')
        && !value.ends_with('-')
        && value.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
}

fn normalize_owned_path(value: &str) -> Result<String, String> {
    let value = value.trim().replace('\\', "/");
    let value = value.trim_start_matches("./").trim_end_matches('/');
    if value.is_empty()
        || value.starts_with('/')
        || value.contains(':')
        || value
            .split('/')
            .any(|part| matches!(part, "" | "." | ".." | ".git" | ".hermes"))
    {
        return Err(format!("unsafe Project Swarm owned path `{value}`"));
    }
    Ok(value.to_string())
}

fn path_is_owned(path: &str, owned: &str) -> bool {
    let path = path.replace('\\', "/");
    let owned = owned.replace('\\', "/").trim_end_matches('/').to_string();
    path == owned || path.starts_with(&format!("{owned}/"))
}

fn changed_paths(workspace: &Path) -> Result<Vec<String>, String> {
    let tracked = git_output(workspace, &["diff", "--name-only", "HEAD"])?;
    let untracked = git_output(workspace, &["ls-files", "--others", "--exclude-standard"])?;
    Ok(tracked
        .lines()
        .chain(untracked.lines())
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(str::to_string)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect())
}

fn require_clean_git_repository(workspace: &Path) -> Result<(), String> {
    git_output(workspace, &["rev-parse", "--show-toplevel"])?;
    let status = git_output(
        workspace,
        &["status", "--porcelain", "--untracked-files=all"],
    )?;
    if !status.trim().is_empty() {
        return Err("Project Swarm coding requires a clean Git project; using the sequential Hermes fallback".to_string());
    }
    Ok(())
}

fn publish_integration(
    workspace: &Path,
    expected_base: &str,
    integration: &Path,
) -> Result<String, String> {
    require_clean_git_repository(workspace)?;
    let current = git_output(workspace, &["rev-parse", "HEAD"])?;
    if current.trim() != expected_base {
        return Err(
            "project changed while Project Swarm was running; integration was not applied"
                .to_string(),
        );
    }
    let integrated = git_output(integration, &["rev-parse", "HEAD"])?;
    git_ok(workspace, &["merge", "--ff-only", integrated.trim()])?;
    Ok(integrated.trim().to_string())
}

fn git_output(workspace: &Path, arguments: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(workspace)
        .output()
        .map_err(|error| format!("could not start Git: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "Git command failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn git_ok(workspace: &Path, arguments: &[&str]) -> Result<(), String> {
    git_output(workspace, arguments).map(|_| ())
}

fn path_text(path: &Path) -> Result<&str, String> {
    path.to_str()
        .ok_or_else(|| "Project Swarm path is not valid UTF-8".to_string())
}

struct ShellResult {
    success: bool,
    output: Vec<u8>,
}

fn run_shell(workspace: &Path, command: &str) -> Result<ShellResult, String> {
    let output = if cfg!(windows) {
        Command::new("cmd.exe")
            .args(["/D", "/C", command])
            .current_dir(workspace)
            .output()
    } else {
        Command::new("sh")
            .args(["-lc", command])
            .current_dir(workspace)
            .output()
    }
    .map_err(|error| format!("could not start integration verification: {error}"))?;
    let mut bytes = output.stdout;
    if !bytes.is_empty() && !output.stderr.is_empty() {
        bytes.extend_from_slice(b"\n--- stderr ---\n");
    }
    bytes.extend_from_slice(&output.stderr);
    Ok(ShellResult {
        success: output.status.success(),
        output: bytes,
    })
}

fn extract_json(value: &str) -> Result<&str, String> {
    let start = value
        .find('{')
        .ok_or("Project Swarm planner returned no JSON object")?;
    let end = value
        .rfind('}')
        .ok_or("Project Swarm planner returned incomplete JSON")?;
    (end >= start)
        .then_some(&value[start..=end])
        .ok_or_else(|| "Project Swarm planner returned incomplete JSON".to_string())
}

fn record(
    path: &Path,
    journal: &mut Vec<JournalEvent>,
    state: &str,
    task: Option<&str>,
    detail: &str,
) -> Result<(), String> {
    journal.push(JournalEvent {
        state: state.to_string(),
        task: task.map(str::to_string),
        detail: detail.to_string(),
    });
    fs::write(
        path,
        serde_json::to_vec_pretty(journal).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("could not update Project Swarm journal: {error}"))
}

fn next_sequence() -> u64 {
    static SEQUENCE: AtomicU64 = AtomicU64::new(20_000);
    SEQUENCE.fetch_add(1, Ordering::Relaxed)
}

fn emit(options: &CoordinatorOptions, event_type: &str, summary: &str, metadata: Value) {
    let _ = options.event_sender.send(json!({
        "sequence": next_sequence(),
        "event": {"type": event_type, "summary": summary, "metadata": metadata}
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn plan(tasks: Vec<CodingTask>) -> CodingPlan {
        CodingPlan {
            summary: "test".to_string(),
            tasks,
            verification_command: "cargo test".to_string(),
        }
    }

    fn task(id: &str, dependencies: &[&str], owned: &[&str]) -> CodingTask {
        CodingTask {
            id: id.to_string(),
            objective: format!("Implement {id}"),
            depends_on: dependencies.iter().map(|value| value.to_string()).collect(),
            owned_paths: owned.iter().map(|value| value.to_string()).collect(),
        }
    }

    #[test]
    fn recognizes_explicit_swarm_requests() {
        assert!(requested("Use Project Swarm for this feature"));
        assert!(requested("Can the agents code in parallel?"));
        assert!(!requested("Create a CRUD API"));
    }

    #[test]
    fn automatically_coordinates_substantial_project_work() {
        assert!(should_coordinate(
            "Create a complete CRUD API for bus ticketing"
        ));
        assert!(should_coordinate(
            "Build a frontend and backend for the booking flow"
        ));
        assert!(should_coordinate(
            "Implement authentication, database, and API tests"
        ));
        assert!(!should_coordinate("Rename the login button"));
        assert!(!should_coordinate("Create a CRUD API with a single worker"));
    }

    #[test]
    fn detects_frontend_requests_for_integration_browser_review() {
        assert!(requires_browser_acceptance("Create a React frontend"));
        assert!(requires_browser_acceptance("Build a web app"));
        assert!(!requires_browser_acceptance("Create a Rust API"));
    }

    #[test]
    fn validates_disjoint_acyclic_plan() {
        assert!(validate_plan(&plan(vec![
            task("api", &[], &["src/api"]),
            task("ui", &[], &["src/ui"]),
            task("tests", &["api", "ui"], &["tests"]),
        ]))
        .is_ok());
    }

    #[test]
    fn rejects_overlapping_ownership_and_cycles() {
        assert!(validate_plan(&plan(vec![
            task("api", &[], &["src"]),
            task("ui", &[], &["src/ui"]),
        ]))
        .is_err());
        assert!(validate_plan(&plan(vec![
            task("api", &["ui"], &["src/api"]),
            task("ui", &["api"], &["src/ui"]),
        ]))
        .is_err());
    }

    #[test]
    fn rejects_unsafe_owned_paths() {
        assert!(normalize_owned_path("../outside").is_err());
        assert!(normalize_owned_path(".git/config").is_err());
        assert_eq!(normalize_owned_path("./src/api/").unwrap(), "src/api");
    }

    #[test]
    fn accepts_only_bounded_verification_commands() {
        assert!(safe_verification_command("npm test"));
        assert!(safe_verification_command("cargo test --workspace"));
        assert!(safe_verification_command("python -m pytest tests/api"));
        assert!(!safe_verification_command("npm test && deploy"));
        assert!(!safe_verification_command("rm -rf project"));
    }

    #[test]
    fn clean_repository_gate_rejects_uncommitted_work() {
        let repository = std::env::temp_dir().join(format!(
            "mundusx-swarm-git-test-{}",
            Uuid::new_v4().simple()
        ));
        fs::create_dir_all(&repository).unwrap();
        git_ok(&repository, &["init", "-q"]).unwrap();
        let mut file = fs::File::create(repository.join("README.md")).unwrap();
        writeln!(file, "initial").unwrap();
        drop(file);
        git_ok(&repository, &["add", "README.md"]).unwrap();
        git_ok(
            &repository,
            &[
                "-c",
                "user.name=Swarm Test",
                "-c",
                "user.email=test@mundusx.invalid",
                "commit",
                "-q",
                "-m",
                "initial",
            ],
        )
        .unwrap();
        assert!(require_clean_git_repository(&repository).is_ok());
        fs::write(repository.join("README.md"), "changed\n").unwrap();
        assert!(require_clean_git_repository(&repository).is_err());
        fs::remove_dir_all(repository).unwrap();
    }

    #[test]
    fn integration_is_invisible_until_atomic_publish() {
        let repository = std::env::temp_dir().join(format!(
            "mundusx-swarm-publish-test-{}",
            Uuid::new_v4().simple()
        ));
        let integration = repository.with_extension("integration");
        fs::create_dir_all(&repository).unwrap();
        git_ok(&repository, &["init", "-q"]).unwrap();
        fs::write(repository.join("value.txt"), "before\n").unwrap();
        git_ok(&repository, &["add", "value.txt"]).unwrap();
        git_ok(
            &repository,
            &[
                "-c",
                "user.name=Swarm Test",
                "-c",
                "user.email=test@mundusx.invalid",
                "commit",
                "-q",
                "-m",
                "base",
            ],
        )
        .unwrap();
        let base = git_output(&repository, &["rev-parse", "HEAD"]).unwrap();
        git_ok(
            &repository,
            &[
                "worktree",
                "add",
                "--detach",
                path_text(&integration).unwrap(),
                base.trim(),
            ],
        )
        .unwrap();
        fs::write(integration.join("value.txt"), "after\n").unwrap();
        git_ok(&integration, &["add", "value.txt"]).unwrap();
        git_ok(
            &integration,
            &[
                "-c",
                "user.name=Swarm Test",
                "-c",
                "user.email=test@mundusx.invalid",
                "commit",
                "-q",
                "-m",
                "integrated",
            ],
        )
        .unwrap();
        assert_eq!(
            fs::read_to_string(repository.join("value.txt"))
                .unwrap()
                .trim(),
            "before"
        );
        publish_integration(&repository, base.trim(), &integration).unwrap();
        assert_eq!(
            fs::read_to_string(repository.join("value.txt"))
                .unwrap()
                .trim(),
            "after"
        );
        git_ok(
            &repository,
            &[
                "worktree",
                "remove",
                "--force",
                path_text(&integration).unwrap(),
            ],
        )
        .unwrap();
        fs::remove_dir_all(repository).unwrap();
    }
}
