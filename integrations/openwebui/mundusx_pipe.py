"""Thin OpenWebUI adapter for the local MundusX agent API."""

from typing import Any
from uuid import NAMESPACE_URL, uuid5

import httpx
from pydantic import BaseModel, Field


class Pipe:
    class Valves(BaseModel):
        MUNDUSX_AGENT_URL: str = Field(default="http://host.docker.internal:11436")
        MUNDUSX_AGENT_API_KEY: str = Field(default="")
        REQUEST_TIMEOUT_SECONDS: int = Field(default=600, ge=1, le=3600)
        ALLOW_MUTATIONS: bool = Field(default=False)

    def __init__(self) -> None:
        self.valves = self.Valves()

    def pipes(self) -> list[dict[str, str]]:
        return [{"id": "mundusx-agent", "name": "MundusX Agent"}]

    async def pipe(
        self,
        body: dict[str, Any],
        __user__: dict[str, Any] | None = None,
        __metadata__: dict[str, Any] | None = None,
    ) -> str:
        metadata = __metadata__ or {}
        user = __user__ or {}
        external_session = str(
            metadata.get("chat_id")
            or body.get("chat_id")
            or f"openwebui:{user.get('id', 'anonymous')}"
        )
        session_id = str(uuid5(NAMESPACE_URL, f"mundusx:{external_session}"))
        payload = dict(body)
        payload["model"] = "mundusx-agent"
        payload["stream"] = False
        headers = {"X-MundusX-Session-Id": session_id}
        if self.valves.ALLOW_MUTATIONS:
            headers["X-MundusX-Allow-Mutations"] = "true"
        if self.valves.MUNDUSX_AGENT_API_KEY:
            headers["Authorization"] = f"Bearer {self.valves.MUNDUSX_AGENT_API_KEY}"
        url = self.valves.MUNDUSX_AGENT_URL.rstrip("/") + "/v1/chat/completions"
        async with httpx.AsyncClient(timeout=self.valves.REQUEST_TIMEOUT_SECONDS) as client:
            response = await client.post(url, json=payload, headers=headers)
            response.raise_for_status()
            result = response.json()
        return result["choices"][0]["message"]["content"]
