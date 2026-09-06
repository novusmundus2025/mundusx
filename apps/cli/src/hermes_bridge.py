"""Structured MundusX bridge for an installed Hermes Agent runtime."""

import json
import os
import sys
import traceback


def emit(prefix, value):
    print(prefix + json.dumps(value, ensure_ascii=False, default=str), flush=True)


def select_project_skills(prompt):
    """Choose a small native Hermes skill set for a project request."""
    text = (prompt or "").lower()
    selected = []

    def add(name):
        if name not in selected:
            selected.append(name)

    if any(word in text for word in ("bug", "debug", "error", "fail", "fix", "broken")):
        add("systematic-debugging")
    if any(word in text for word in ("test", "tests", "tdd", "implement", "create", "build", "code", "app")):
        add("test-driven-development")
    if any(word in text for word in ("plan", "design", "architecture", "refactor", "migrate")):
        add("plan")
    if any(word in text for word in ("existing", "repository", "repo", "codebase", "inspect", "understand")):
        add("codebase-inspection")
    return selected[:3]


def main():
    project_root = os.environ["MUNDUSX_HERMES_PROJECT_ROOT"]
    sys.path.insert(0, project_root)

    from run_agent import AIAgent

    def event_callback(kind, data=None):
        payload = data if isinstance(data, dict) else {"value": data}
        emit("MUNDUSX_EVENT=", {"type": str(kind), "data": payload})

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
        # Project work uses Hermes' native coding tools and its progressive-
        # disclosure SKILL.md loader.  Keeping this explicit avoids enabling
        # unrelated integrations while still allowing Hermes to discover and
        # load the right development workflow for the request.
        enabled_toolsets=["coding", "skills"],
        quiet_mode=True,
        tool_progress_mode="all",
        event_callback=event_callback,
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
