"""Structured MundusX bridge for an installed Hermes Agent runtime."""

import json
import os
import re
import sys
import traceback
import time


class AnswerStream:
    """Bounded preview of the current model turn, not the full tool transcript."""
    def __init__(self, publish, clock=time.monotonic):
        self.publish, self.clock = publish, clock
        self.text, self.last, self.dirty, self.truncated = "", 0, False, False

    def delta(self, text):
        if not isinstance(text, str) or not text:
            return
        self.text += text
        if len(self.text) > 32768:
            self.text = self.text[-32768:]
            self.truncated = True
        self.dirty = True
        if self.clock() - self.last >= 0.5:
            self.flush()

    def flush(self):
        if self.dirty:
            self.publish({"type": "assistant_snapshot", "data": {"text": self.text, "truncated": self.truncated}})
            self.last, self.dirty = self.clock(), False

    def reset(self):
        """Clear narration once its tool starts so later turns cannot accumulate."""
        self.text, self.dirty, self.truncated = "", False, False
        self.last = self.clock()
        self.publish({"type": "assistant_snapshot", "data": {"text": "", "truncated": False}})


def emit(prefix, value):
    print(prefix + json.dumps(value, ensure_ascii=False, default=str), flush=True)


SKILL_DISCOVERY_GUIDANCE = """For this project task, discover relevant installed Hermes skills using skills_list
when needed, and load applicable instructions with skill_view before using them.
Use only skills relevant to the task; do not load the entire library or invent missing skills.
Project instructions and the current user request still apply. A skill does not grant extra permissions.
"""

RECOVERY_GUIDANCE = """Resume this interrupted project task from the preserved Hermes session, recovery checkpoint, and current workspace.
Treat completed checkpoint boundaries and existing files as authoritative. Continue with the first unfinished implementation or verification step.
Do not repeat skill discovery, skill loading, web research, or unchanged-file inspection that the preserved session or checkpoint already records as successful unless a concrete error requires it.
If the requested edits are already present, run the required build, test, lint, or browser acceptance checks and finish with a concise result.
"""

EXECUTION_EFFICIENCY_GUIDANCE = """Work directly and keep model turns economical.
For new test apps and apps intended to run locally, default to SQLite when persistent storage is needed and the user has not explicitly specified a storage technology.
Use a persistent SQLite file with the project's existing language and framework. Do not introduce Docker or an external database service solely for this default.
Honor explicitly requested storage technologies and preserve existing projects' database choices. This default does not select storage for production deployments or trigger database migrations.
Do not narrate each intended read, edit, or command before calling a tool.
Inspect each unchanged file only once, batch related operations when practical, and do not repeat a completed step.
Use the structured tool progress events for status. Reserve prose for a concise final summary after implementation and verification.
Tool results and JSON representations escape real newline characters as \\n. Do not treat that display escaping as proof that a source file contains literal backslash-n text. If syntax is uncertain, run the project build or parser once and trust the result; do not repeatedly inspect the same bytes after a successful build.
For multi-file code generation, reserve the final tool turns for dependency installation and a real build, test, lint, syntax, or smoke check. Group related work where the tools permit it; do not consume the entire budget writing one file per planning cycle.
When the prompt contains MUNDUSX_REUSABLE_TEST_ACCEPTANCE_V1, create or update a maintainable project-owned regression test for the requested behavior or reproduced failure and run the test suite after the final implementation change. Prefer the existing framework. A build, lint command, generated output, one-off probe, or manual browser check is not a reusable regression test.
For frontend work, reconcile every third-party source import with the package manifest that owns that source tree. Install missing dependencies in that package directory, not an unrelated parent package. Treat missing-export, unresolved-import, and equivalent bundler diagnostics as build failures even when a bundler exits with status 0. Run that frontend package's production build successfully before starting browser acceptance. Then render the changed flow in a browser, inspect the console for runtime exceptions, and exercise the requested interaction. If either check fails, repair the source and repeat both checks; never replace frontend runtime acceptance with a passing backend test.
After the requested change and required build, lint, test, or browser acceptance checks succeed, return the final response immediately. Do not start another inspection cycle or add unrelated improvements.
"""

STANDARD_MAX_ITERATIONS = 12
COMPREHENSIVE_MAX_ITERATIONS = 24
FRONTEND_MAX_ITERATIONS = 24
SWARM_PLANNER_MARKER = "MUNDUSX_SWARM_PLANNER_V1"
PROJECT_COMPACTION_TOKENS = 16_384
CHECKPOINT_EVENT_LIMIT = 16
CHECKPOINT_TARGET_LIMIT = 24


def project_iteration_budget(prompt):
    """Keep small tasks quick while giving multi-file builds room to verify."""
    value = str(prompt or "").lower()
    comprehensive_signals = (
        "complete", "full project", "from scratch", "crud", "api",
        "frontend", "react", "website", "application", "all files",
    )
    signal_count = sum(signal in value for signal in comprehensive_signals)
    if FRONTEND_ACCEPTANCE_MARKER in value:
        return FRONTEND_MAX_ITERATIONS
    if signal_count >= 2:
        return COMPREHENSIVE_MAX_ITERATIONS
    return STANDARD_MAX_ITERATIONS


def project_user_prompt(raw_prompt, recovery_note=""):
    """Compose a fresh-task or recovery prompt without replaying completed setup."""
    task_prompt = raw_prompt
    if recovery_note:
        task_prompt = recovery_note + "\n\n" + task_prompt
    guidance = RECOVERY_GUIDANCE if recovery_note else SKILL_DISCOVERY_GUIDANCE
    return (
        guidance
        + "\n"
        + EXECUTION_EFFICIENCY_GUIDANCE
        + "\nUser project request:\n"
        + task_prompt
    )


class RecoveryCheckpoint:
    """Bounded durable milestone journal for safe cross-attempt recovery."""

    def __init__(self, workspace, task_id):
        safe_id = re.sub(r"[^A-Za-z0-9_.-]+", "-", task_id or "project-task")[:96]
        self.path = os.path.join(workspace, ".hermes", "checkpoints", safe_id + ".json")
        self.previous = self._load()
        self.events = list((self.previous or {}).get("events") or [])
        self.summary = self._normalized_summary((self.previous or {}).get("summary"))
        try:
            self.attempts = max(0, int((self.previous or {}).get("attempts") or 0))
        except (TypeError, ValueError):
            self.attempts = 0
        if (self.previous or {}).get("schema_version", 1) < 2:
            for event in self.events:
                self._summarize(event)

    @staticmethod
    def _normalized_summary(value):
        def count(raw):
            try:
                return max(0, int(raw or 0))
            except (TypeError, ValueError):
                return 0

        value = value if isinstance(value, dict) else {}
        activities = value.get("activities") if isinstance(value.get("activities"), dict) else {}
        targets = value.get("recent_targets") if isinstance(value.get("recent_targets"), list) else []
        return {
            "completed_boundaries": count(value.get("completed_boundaries")),
            "successful_boundaries": count(value.get("successful_boundaries")),
            "failed_boundaries": count(value.get("failed_boundaries")),
            "verification_passes": count(value.get("verification_passes")),
            "verification_failures": count(value.get("verification_failures")),
            "activities": {
                str(name)[:64]: count(raw_count)
                for name, raw_count in list(activities.items())[:24]
            },
            "recent_targets": [str(target)[-512:] for target in targets[-CHECKPOINT_TARGET_LIMIT:]],
        }

    def _summarize(self, event):
        self.summary["completed_boundaries"] += 1
        success = event.get("success")
        if success is True:
            self.summary["successful_boundaries"] += 1
        elif success is False:
            self.summary["failed_boundaries"] += 1
        if event.get("verification"):
            if success is True:
                self.summary["verification_passes"] += 1
            elif success is False:
                self.summary["verification_failures"] += 1
        activity = str(event.get("activity") or "other")[:64]
        self.summary["activities"][activity] = self.summary["activities"].get(activity, 0) + 1
        target = event.get("target")
        if target:
            targets = [item for item in self.summary["recent_targets"] if item != target]
            targets.append(target)
            self.summary["recent_targets"] = targets[-CHECKPOINT_TARGET_LIMIT:]

    def _load(self):
        try:
            with open(self.path, "r", encoding="utf-8") as handle:
                value = json.load(handle)
            return value if isinstance(value, dict) else None
        except (OSError, ValueError):
            return None

    def recovery_note(self):
        if not self.previous or self.previous.get("state") != "active":
            return ""
        events = self.previous.get("events") or []
        compact = [
            {
                "tool": event.get("tool"),
                "activity": event.get("activity"),
                "success": event.get("success"),
                "target": event.get("target"),
            }
            for event in events[-12:]
        ]
        return (
            "Recovery checkpoint from an interrupted run. Treat the workspace as authoritative, "
            "reconcile this bounded milestone summary, and continue without repeating completed work: "
            + json.dumps(
                {
                    "attempts": self.previous.get("attempts", 1),
                    "milestone_summary": self.previous.get("summary") or {},
                    "recent_boundaries": compact,
                },
                separators=(",", ":"),
            )
        )

    def _write(self, state):
        directory = os.path.dirname(self.path)
        os.makedirs(directory, exist_ok=True)
        payload = {
            "schema_version": 2,
            "state": state,
            "updated_unix_ms": int(time.time() * 1000),
            "attempts": self.attempts,
            "summary": self.summary,
            "events": self.events[-CHECKPOINT_EVENT_LIMIT:],
        }
        temporary = self.path + ".tmp-" + str(os.getpid())
        with open(temporary, "w", encoding="utf-8") as handle:
            json.dump(payload, handle, separators=(",", ":"))
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, self.path)

    def start(self):
        self.attempts += 1
        self._write("active")

    def record(self, name, arguments, result):
        target = None
        if isinstance(arguments, dict):
            for key in ("path", "file_path", "filename"):
                value = arguments.get(key)
                if isinstance(value, str) and value:
                    target = value[-512:]
                    break
        command = arguments.get("command") if isinstance(arguments, dict) else None
        event = {
            "tool": name,
            "activity": tool_activity(name, arguments),
            "success": tool_outcome(result, name),
            "verification": is_verification_command(command),
            "target": target,
        }
        self.events.append(event)
        self._summarize(event)
        self._write("active")

    def finish(self, succeeded):
        self._write("completed" if succeeded else "active")


def configure_project_compaction(agent):
    """Cap Hermes' token-aware preflight without falsifying model metadata."""
    compressor = getattr(agent, "context_compressor", None)
    if compressor is None:
        return False
    compressor.threshold_tokens_cap = PROJECT_COMPACTION_TOKENS
    compressor.threshold_tokens = min(
        int(compressor.threshold_tokens), PROJECT_COMPACTION_TOKENS
    )
    return True


def child_workspace_path(path, platform=os.name):
    """Remove Windows verbatim prefixes that Git Bash cannot translate."""
    if platform != "nt":
        return path
    if path.startswith("\\\\?\\UNC\\"):
        return "\\\\" + path[8:]
    if path.startswith("\\\\?\\"):
        return path[4:]
    return path


FRONTEND_ACCEPTANCE_MARKER = "MUNDUSX_FRONTEND_ACCEPTANCE_V1"
BROWSER_ACCEPTANCE_EVIDENCE_MARKER = "MUNDUSX_BROWSER_ACCEPTANCE_V1"


def loaded_skill(name, result):
    if name != "skill_view":
        return None
    if isinstance(result, str):
        try:
            result = json.loads(result)
        except (ValueError, TypeError):
            return None
    if isinstance(result, dict) and result.get("success") is True and not result.get("error"):
        value = result.get("name")
        if isinstance(value, str) and value.strip():
            return value[:160]
    return None


def is_verification_command(command):
    value = " " + str(command or "").lower().strip() + " "
    checks = (
        " npm test ", " npm run test ", " pnpm test ", " yarn test ",
        " pytest ", " python -m pytest ", " cargo test ", " go test ",
        " dotnet test ", " mvn test ", " gradle test ", " gradlew test ",
        " npm run build ", " pnpm build ", " yarn build ", " cargo build ",
        " npm run lint ", " pnpm lint ", " yarn lint ",
        " node --test ", " node --check ", " npm run check ",
        " npx tsc ", " python -m compileall ",
    )
    package_check = re.search(
        r"(?:^|[|;&])\s*(?:npm|pnpm|yarn)(?:\s+(?:--prefix|-c|--dir)\s+\S+)*\s+(?:run\s+)?(?:build|test|lint|check)\b",
        str(command or "").lower(),
    )
    return package_check is not None or any(check in value for check in checks)


def tool_result_succeeded(result):
    if isinstance(result, dict):
        if result.get("success") is False or result.get("is_error") is True:
            return False
        code = result.get("exit_code", result.get("returncode"))
        if code is not None:
            return int(code) == 0
    text = str(result or "").lower()
    return not any(marker in text for marker in ('"success": false', '"exit_code": 1', "traceback (most recent call last)"))


def tool_activity(name, arguments):
    """Report observed tool intent without exposing commands, credentials or output."""
    name = str(name or "").lower()
    if name == "skills_list":
        return "skill_discovery"
    if name == "skill_view":
        return "skill_load"
    if name in ("terminal", "execute", "shell"):
        command = str(arguments.get("command", "") if isinstance(arguments, dict) else "").lower()
        prefix = r"(?:^|[|;&])\s*"
        package_command = r"(?:npm|pnpm|yarn)(?:\s+(?:--prefix|-c|--dir)\s+\S+)*\s+"
        if re.search(prefix + r"(?:" + package_command + r"(?:run\s+)?build|cargo build)\b", command):
            return "build"
        if re.search(prefix + r"(?:pytest|python -m pytest|cargo test|" + package_command + r"(?:run\s+)?test|mvn test|go test|dotnet test)\b", command):
            return "test"
        if re.search(prefix + r"(?:" + package_command + r"(?:run\s+)?lint|ruff check)\b", command):
            return "lint"
        if re.search(prefix + r"(?:node --test|node --check|" + package_command + r"(?:run\s+)?check|npx tsc|python -m compileall)\b", command):
            return "test"
        if re.search(prefix + r"(?:npm install|npm ci|pnpm install|yarn install|pip install)\b", command):
            return "dependencies"
        if re.search(prefix + r"(?:node|python|python3)\s", command):
            return "run"
        return "command"
    if name in ("write_file", "patch", "apply_patch", "patch_file", "write"):
        return "write"
    if name == "execute_code" or name.startswith("browser_"):
        return "browser_acceptance"
    if name in ("read_file", "read", "search_files", "grep", "glob", "search", "list_directory"):
        return "inspect"
    if name.startswith("browser_") or name in ("web_search", "web_fetch"):
        return "research"
    return "tool"


def tool_outcome(result, name=None):
    if isinstance(result, str):
        try:
            result = json.loads(result)
        except (ValueError, TypeError):
            return None
    if not isinstance(result, dict):
        return None
    if result.get("error") or result.get("is_error") is True or result.get("success") is False:
        return False
    output = str(result.get("output", result.get("stderr", "")) or "").lower()
    fatal_build_diagnostics = (
        " is not exported by ",
        " does not provide an export named ",
        "module has no exported member",
        "could not resolve import",
        "failed to resolve import",
    )
    if any(marker in output for marker in fatal_build_diagnostics):
        return False
    code = result.get("exit_code", result.get("returncode"))
    if code is not None:
        try:
            return int(code) == 0
        except (ValueError, TypeError):
            return None
    if result.get("success") is True or result.get("is_error") is False:
        return True
    # Recognize Hermes' typed file results, including empty files and searches.
    # A successful write is not proof that a subsequent build or test passes.
    if name == "read_file" and isinstance(result.get("content"), str) and "total_lines" in result:
        return True
    if name == "write_file" and type(result.get("bytes_written")) is int and result["bytes_written"] >= 0:
        return True
    if name == "search_files" and type(result.get("total_count")) is int and result["total_count"] >= 0:
        return True
    return None


def browser_acceptance_evidence(result):
    if isinstance(result, str):
        try:
            result = json.loads(result)
        except (ValueError, TypeError):
            normalized = re.sub(r"\s+", "", result).lower()
            return all(token in normalized for token in (
                '"mundusx_browser_acceptance_v1":true',
                '"rendered":true',
                '"flow_exercised":true',
                '"console_errors":[]',
                '"desktop_checked":true',
                '"narrow_checked":true',
            ))
    if isinstance(result, list):
        return any(browser_acceptance_evidence(value) for value in result)
    if not isinstance(result, dict):
        return False
    if (
        result.get(BROWSER_ACCEPTANCE_EVIDENCE_MARKER) is True
        and result.get("rendered") is True
        and result.get("flow_exercised") is True
        and result.get("console_errors") == []
        and result.get("desktop_checked") is True
        and result.get("narrow_checked") is True
    ):
        return True
    return any(browser_acceptance_evidence(value) for value in result.values())


def browser_verification(name, arguments, result):
    if tool_outcome(result, name) is not True:
        return None
    native = {
        "browser_navigate": "render",
        "browser_snapshot": "snapshot",
        "browser_vision": "snapshot",
        "browser_console": "console",
        "browser_exec": "suite",
    }.get(name)
    if native:
        return native
    if name == "execute_code" and browser_acceptance_evidence(result):
        return "suite"
    return None


def main():
    project_root = os.environ["MUNDUSX_HERMES_PROJECT_ROOT"]
    workspace = child_workspace_path(
        os.path.realpath(os.environ.get("MUNDUSX_HERMES_WORKSPACE", os.getcwd()))
    )
    if not os.path.isdir(workspace):
        raise RuntimeError("MundusX project workspace is unavailable")
    # The embedded bridge bypasses Hermes' CLI bootstrap, which normally pins
    # terminal and file tools to the launch directory. Pin the project here so
    # a saved Hermes config (for example terminal.cwd = the user's home) cannot
    # redirect project writes outside the selected MundusX project.
    os.chdir(workspace)
    os.environ["TERMINAL_CWD"] = workspace
    sys.path.insert(0, project_root)

    raw_user_prompt = os.environ["MUNDUSX_HERMES_PROMPT"]
    planner_mode = SWARM_PLANNER_MARKER in raw_user_prompt
    task_id = os.environ.get("MUNDUSX_HERMES_TASK") or os.environ.get("MUNDUSX_HERMES_SESSION") or "project-task"
    checkpoint_root = os.environ.get("HERMES_HOME", workspace) if planner_mode else workspace
    checkpoint = RecoveryCheckpoint(checkpoint_root, task_id)
    recovery_note = checkpoint.recovery_note()
    checkpoint.start()

    from run_agent import AIAgent
    from agent.runtime_cwd import set_session_cwd
    from tools.terminal_tool import register_task_env_overrides

    task_id = os.environ.get("MUNDUSX_HERMES_TASK") or "default"
    # Preserve the installed runtime's project-directory binding on recovery.
    set_session_cwd(workspace)
    register_task_env_overrides(task_id, {"cwd": workspace})

    selected_skills = []
    answer_stream = AnswerStream(lambda event: emit("MUNDUSX_EVENT=", event))
    # Reset a previous attempt's preview when a recovery starts a new run.
    emit("MUNDUSX_EVENT=", {"type": "assistant_snapshot", "data": {"text": "", "truncated": False}})

    def event_callback(kind, data=None):
        payload = data if isinstance(data, dict) else {"value": data}
        emit("MUNDUSX_EVENT=", {"type": str(kind), "data": payload})

    def tool_start_callback(call_id, name, arguments):
        answer_stream.flush()
        answer_stream.reset()
        command = arguments.get("command") if isinstance(arguments, dict) else None
        emit(
            "MUNDUSX_EVENT=",
            {
                "type": "tool_started",
                "data": {
                    "call_id": call_id,
                    "name": name,
                    "verification": is_verification_command(command),
                    "activity": tool_activity(name, arguments),
                },
            },
        )

    def tool_complete_callback(call_id, name, arguments, result):
        skill = loaded_skill(name, result)
        if skill and skill not in selected_skills:
            selected_skills.append(skill)
            emit("MUNDUSX_EVENT=", {"type": "skills_selected", "data": {"skills": [skill]}})
        command = arguments.get("command") if isinstance(arguments, dict) else None
        checkpoint.record(name, arguments, result)
        emit(
            "MUNDUSX_EVENT=",
            {
                "type": "tool_completed",
                "data": {
                    "call_id": call_id,
                    "name": name,
                    "verification": is_verification_command(command),
                    "browser_verification": browser_verification(name, arguments, result),
                    "success": tool_outcome(result, name),
                    "activity": tool_activity(name, arguments),
                },
            },
        )

    session_id = os.environ.get("MUNDUSX_HERMES_SESSION") or None
    user_prompt = raw_user_prompt
    max_iterations = 1 if planner_mode else project_iteration_budget(user_prompt)
    if planner_mode:
        enabled_toolsets = []
    else:
        user_prompt = project_user_prompt(user_prompt, recovery_note)
        enabled_toolsets = ["coding", "skills"]
        if FRONTEND_ACCEPTANCE_MARKER in user_prompt:
            enabled_toolsets.extend(["browser", "browser-use"])
    agent = AIAgent(
        base_url=os.environ["OPENAI_BASE_URL"],
        api_key=os.environ["OPENAI_API_KEY"],
        provider="openai-api",
        model="mundusx-agnostic",
        max_iterations=max_iterations,
        enabled_toolsets=enabled_toolsets,
        quiet_mode=True,
        tool_progress_mode="all",
        event_callback=event_callback,
        tool_start_callback=tool_start_callback,
        tool_complete_callback=tool_complete_callback,
        stream_delta_callback=answer_stream.delta,
        session_id=session_id,
        skip_memory=True,
        load_soul_identity=False,
    )
    # Publish the runtime session before the first model request. The parent
    # persists this immediately, so a watchdog termination can resume the same
    # Hermes history instead of creating a new history=0 session.
    emit(
        "MUNDUSX_EVENT=",
        {
            "type": "runtime_session_ready",
            "data": {"session_id": getattr(agent, "session_id", None) or session_id},
        },
    )
    configure_project_compaction(agent)
    try:
        result = agent.run_conversation(
            user_message=user_prompt,
            task_id=task_id,
        )
    except Exception as error:
        answer_stream.flush()
        emit(
            "MUNDUSX_RESULT=",
            {
                "final_response": "",
                "failed": True,
                "partial": True,
                "error": str(error),
                "session_id": getattr(agent, "session_id", None) or session_id,
                "tool_calls": [],
                "turn_count": 0,
                "skills": selected_skills,
            },
        )
        traceback.print_exc(file=sys.stderr)
        return 1
    answer_stream.flush()
    messages = result.get("messages") or []
    tool_calls = []
    for message in messages:
        for call in message.get("tool_calls") or []:
            function = call.get("function") or {}
            tool_calls.append({"name": function.get("name"), "id": call.get("id")})
    emit(
        "MUNDUSX_RESULT=",
        {
            "final_response": result.get("final_response") or "",
            "failed": bool(result.get("failed")),
            "partial": bool(result.get("partial")),
            "error": result.get("error"),
            "interrupted": bool(result.get("interrupted")),
            "session_id": getattr(agent, "session_id", None) or session_id,
            "tool_calls": tool_calls,
            "turn_count": len(messages),
            "skills": selected_skills,
        },
    )
    checkpoint.finish(
        not result.get("failed") and not result.get("interrupted")
    )
    return 1 if result.get("failed") and not result.get("final_response") else 0


if __name__ == "__main__":
    try:
        exit_code = main()
    except Exception as error:
        emit(
            "MUNDUSX_RESULT=",
            {"final_response": "", "failed": True, "partial": False, "error": str(error)},
        )
        traceback.print_exc(file=sys.stderr)
        raise SystemExit(1)
    raise SystemExit(exit_code)
