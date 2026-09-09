"""Structured MundusX bridge for an installed Hermes Agent runtime."""

import json
import os
import re
import sys
import traceback
import time


class AnswerStream:
    """Bounded, replaceable previews; replay never appends duplicate text."""
    def __init__(self, publish, clock=time.monotonic):
        self.publish, self.clock = publish, clock
        self.text, self.last, self.dirty, self.truncated = "", 0, False, False

    def delta(self, text):
        if not isinstance(text, str) or not text:
            return
        self.text += text
        if len(self.text) > 8192:
            self.text = self.text[-8192:]
            self.truncated = True
        self.dirty = True
        if self.clock() - self.last >= 0.5:
            self.flush()

    def flush(self):
        if self.dirty:
            self.publish({"type": "assistant_snapshot", "data": {"text": self.text, "truncated": self.truncated}})
            self.last, self.dirty = self.clock(), False


def emit(prefix, value):
    print(prefix + json.dumps(value, ensure_ascii=False, default=str), flush=True)


def select_project_skills(prompt):
    """Choose a small native Hermes skill set for a project request."""
    text = (prompt or "").lower()
    selected = ["codebase-inspection"]

    def add(name):
        if name not in selected:
            selected.append(name)

    if any(word in text for word in ("bug", "debug", "error", "fail", "fix", "broken")):
        add("systematic-debugging")
    if any(word in text for word in ("test", "tests", "tdd", "implement", "create", "build", "code", "app", "add", "function", "index")):
        add("test-driven-development")
    if any(word in text for word in ("plan", "design", "architecture", "refactor", "migrate")):
        add("plan")
    if any(word in text for word in ("existing", "repository", "repo", "codebase", "inspect", "understand")):
        add("codebase-inspection")
    return selected[:3]


def is_verification_command(command):
    value = " " + str(command or "").lower().strip() + " "
    checks = (
        " npm test ", " npm run test ", " pnpm test ", " yarn test ",
        " pytest ", " python -m pytest ", " cargo test ", " go test ",
        " dotnet test ", " mvn test ", " gradle test ", " gradlew test ",
        " npm run build ", " pnpm build ", " yarn build ", " cargo build ",
        " npm run lint ", " pnpm lint ", " yarn lint ",
    )
    return any(check in value for check in checks)


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
    if name in ("terminal", "execute", "shell"):
        command = str(arguments.get("command", "") if isinstance(arguments, dict) else "").lower()
        prefix = r"(?:^|[|;&])\s*"
        if re.search(prefix + r"(?:(?:npm|pnpm|yarn)\s+(?:run\s+)?build|cargo build)\b", command):
            return "build"
        if re.search(prefix + r"(?:pytest|python -m pytest|cargo test|npm test|npm run test|pnpm test|yarn test|mvn test|go test|dotnet test)\b", command):
            return "test"
        if re.search(prefix + r"(?:npm run lint|pnpm lint|yarn lint|ruff check)\b", command):
            return "lint"
        if re.search(prefix + r"(?:npm install|npm ci|pnpm install|yarn install|pip install)\b", command):
            return "dependencies"
        if re.search(prefix + r"(?:node|python|python3)\s", command):
            return "run"
        return "command"
    if name in ("write_file", "patch", "apply_patch", "patch_file", "write"):
        return "write"
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


def main():
    project_root = os.environ["MUNDUSX_HERMES_PROJECT_ROOT"]
    sys.path.insert(0, project_root)

    from run_agent import AIAgent

    answer_stream = AnswerStream(lambda event: emit("MUNDUSX_EVENT=", event))
    # Reset a previous attempt's preview when a recovery starts a new run.
    emit("MUNDUSX_EVENT=", {"type": "assistant_snapshot", "data": {"text": "", "truncated": False}})

    def event_callback(kind, data=None):
        payload = data if isinstance(data, dict) else {"value": data}
        emit("MUNDUSX_EVENT=", {"type": str(kind), "data": payload})

    def tool_start_callback(call_id, name, arguments):
        answer_stream.flush()
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
        command = arguments.get("command") if isinstance(arguments, dict) else None
        emit(
            "MUNDUSX_EVENT=",
            {
                "type": "tool_completed",
                "data": {
                    "call_id": call_id,
                    "name": name,
                    "verification": is_verification_command(command),
                    "success": tool_outcome(result, name),
                    "activity": tool_activity(name, arguments),
                },
            },
        )

    session_id = os.environ.get("MUNDUSX_HERMES_SESSION") or None
    user_prompt = os.environ["MUNDUSX_HERMES_PROMPT"]
    selected_skills = select_project_skills(user_prompt)
    if selected_skills:
        try:
            from agent.skill_commands import build_preloaded_skills_prompt

            skill_prompt, loaded_skills, missing_skills = build_preloaded_skills_prompt(
                selected_skills,
                task_id=os.environ.get("MUNDUSX_HERMES_TASK") or session_id,
            )
            selected_skills = loaded_skills
            if skill_prompt:
                user_prompt = skill_prompt + "\n\nUser project request:\n" + user_prompt
            emit(
                "MUNDUSX_EVENT=",
                {
                    "type": "skills_selected",
                    "data": {"skills": loaded_skills, "missing": missing_skills},
                },
            )
        except Exception as error:
            # Skill discovery must not prevent the coding harness from running.
            emit(
                "MUNDUSX_EVENT=",
                {"type": "skills_unavailable", "data": {"error": str(error)}},
            )
            selected_skills = []
    agent = AIAgent(
        base_url=os.environ["OPENAI_BASE_URL"],
        api_key=os.environ["OPENAI_API_KEY"],
        provider="openai-api",
        model="mundusx-agnostic",
        # Relevant SKILL.md content is preloaded above. Enabling Hermes' global
        # skills toolset here also injects the complete installed skill catalog
        # into every model turn, which can overflow smaller routed workers.
        enabled_toolsets=["coding"],
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
    try:
        result = agent.run_conversation(
            user_message=user_prompt,
            task_id=os.environ.get("MUNDUSX_HERMES_TASK") or session_id,
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
