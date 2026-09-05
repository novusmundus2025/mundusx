"""Structured MundusX bridge for an installed Hermes Agent runtime."""

import json
import os
import sys
import traceback


def emit(prefix, value):
    print(prefix + json.dumps(value, ensure_ascii=False, default=str), flush=True)


def main():
    project_root = os.environ["MUNDUSX_HERMES_PROJECT_ROOT"]
    sys.path.insert(0, project_root)

    from run_agent import AIAgent

    def event_callback(kind, data=None):
        payload = data if isinstance(data, dict) else {"value": data}
        emit("MUNDUSX_EVENT=", {"type": str(kind), "data": payload})

    session_id = os.environ.get("MUNDUSX_HERMES_SESSION") or None
    agent = AIAgent(
        base_url=os.environ["OPENAI_BASE_URL"],
        api_key=os.environ["OPENAI_API_KEY"],
        provider="openai-api",
        model="mundusx-agnostic",
        enabled_toolsets=["coding"],
        quiet_mode=True,
        tool_progress_mode="all",
        event_callback=event_callback,
        session_id=session_id,
        skip_memory=True,
        load_soul_identity=False,
    )
    result = agent.run_conversation(
        user_message=os.environ["MUNDUSX_HERMES_PROMPT"],
        task_id=os.environ.get("MUNDUSX_HERMES_TASK") or session_id,
    )
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
