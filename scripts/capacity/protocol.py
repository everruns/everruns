"""Actual product request paths. Credentials stay in a private operator manifest.

The driver never emits request/response bodies or credentials in results. An
explicit isolated-target assertion and llmsim preflight precede all mutations.
"""

from __future__ import annotations

import json
import asyncio
import hashlib
import os
from dataclasses import dataclass, field
from pathlib import Path
from urllib.parse import urlsplit


@dataclass
class Actor:
    name: str
    path: str
    base: str
    headers: dict[str, str] = field(repr=False)
    create: dict
    org: str
    agent: str = ""
    model: str = ""
    channel: str = ""
    session: str | None = None
    refreshes: int = 0
    refresh_lock: asyncio.Lock = field(default_factory=asyncio.Lock, repr=False)

    @property
    def root(self):
        return self.base.rstrip("/") + ("/v1" if self.path == "console" else "")

    def url(self, suffix):
        return self.root + "/" + suffix.lstrip("/")

    def send(self, body, key):
        if self.path == "console":
            return self.url("commands/create_message"), {
                "params": {"session_id": self.session, "client_message_id": key, **body}
            }
        return self.url(f"sessions/{self.session}/messages"), body

    def polls(self):
        if self.path == "work-chat":
            # Match the host: two authenticated reads every 4s and config every 10s.
            return [
                (4, [f"sessions/{self.session}", f"sessions/{self.session}/work"]),
                (10, ["chat/config"]),
            ]
        if self.path == "console":
            return [
                (
                    15,
                    [
                        "sessions?source=chat&limit=100&mine=true&side_chats_only=true&order=last_activity&agent_id="
                        + self.agent
                    ],
                )
            ]
        return []


def trust_environment(url):
    """Local fixtures connect directly; remote operators retain required proxies."""
    return urlsplit(url).hostname not in {"localhost", "127.0.0.1", "::1"}


@dataclass
class Target:
    actors: list[Actor]
    control: str
    control_headers: dict[str, str]
    model: str
    agent: str
    metadata: dict = field(default_factory=dict)

    def headers_for(self, actor):
        return {**self.control_headers, "X-Org-Id": actor.org}


def check_url(value):
    parsed = urlsplit(value)
    if parsed.username or parsed.password or parsed.query or parsed.fragment:
        raise ValueError("target URLs cannot contain credentials, queries or fragments")
    local = parsed.hostname in {"localhost", "127.0.0.1", "::1"} or (
        parsed.hostname or ""
    ).endswith(".localhost")
    if parsed.scheme != "https" and not (local and parsed.scheme == "http"):
        raise ValueError("remote targets require HTTPS")
    if parsed.hostname in {"app.everruns.com", "chat.everruns.com"} or (
        parsed.hostname or ""
    ).endswith(".chat.everruns.com"):
        raise ValueError("production is not a capacity-test target")


def load_target(path):
    source = Path(path)
    if os.name != "nt" and source.stat().st_mode & 0o077:
        raise ValueError("credential manifest must have mode 0600")
    data = json.loads(source.read_text())
    if data.get("isolated") is not True:
        raise ValueError("operator must identify an isolated target")
    check_url(data["management_url"])
    actors = []
    for row in data["actors"]:
        check_url(row["base"])
        if row["path"] not in {"console", "agent-api", "work-chat"}:
            raise ValueError("unknown product path")
        headers = row.get("headers", {})
        if not any(k.lower() in {"authorization", "cookie"} for k in headers):
            raise ValueError("each actor requires authentication")
        actors.append(
            Actor(
                row["name"],
                row["path"],
                row["base"],
                headers,
                row.get("create", {}),
                row["org"],
                row.get("agent_id", data["agent_id"]),
                row.get("model_id", data["model_id"]),
                row.get("channel_id", ""),
            )
        )
    if not actors or len({a.name for a in actors}) != len(actors):
        raise ValueError("actors must have unique names")
    # Metadata is an allowlist, never a dump of the credential manifest.
    metadata = {
        k: data.get("metadata", {}).get(k)
        for k in ["label", "revision", "topology", "limits", "generator_topology"]
    }
    return Target(
        actors,
        data["management_url"].rstrip("/"),
        data["management_headers"],
        data["model_id"],
        data["agent_id"],
        metadata,
    )


async def preflight(client, target, actors):
    # Validate each tenant, not merely the first actor's simulation setting.
    checked = set()
    channels = set()
    for actor in actors:
        identity = (actor.org, actor.model, actor.agent)
        headers = target.headers_for(actor)
        if identity not in checked:
            model = (
                (
                    await client.get(
                        f"{target.control}/v1/models/{actor.model}", headers=headers
                    )
                )
                .raise_for_status()
                .json()
            )
            provider = (
                (
                    await client.get(
                        f"{target.control}/v1/providers/{model['provider_id']}",
                        headers=headers,
                    )
                )
                .raise_for_status()
                .json()
            )
            if provider["provider_type"] != "llmsim" or not model["enabled"]:
                raise ValueError("capacity testing requires an enabled llmsim model")
            agent = (
                (
                    await client.get(
                        f"{target.control}/v1/agents/{actor.agent}", headers=headers
                    )
                )
                .raise_for_status()
                .json()
            )
            if agent.get("default_model_id") != actor.model:
                raise ValueError("agent must use the verified llmsim model")
            checked.add(identity)
        if actor.path == "console":
            if (
                actor.create.get("agent_id") != actor.agent
                or actor.create.get("model_id", actor.model) != actor.model
            ):
                raise ValueError("console session must use the verified agent/model")
        else:
            if not actor.channel:
                raise ValueError("execution targets require their channel id")
            channel = (actor.org, actor.agent, actor.channel)
            if channel not in channels:
                (
                    await client.get(
                        f"{target.control}/v1/agents/{actor.agent}/channels/{actor.channel}",
                        headers=headers,
                    )
                ).raise_for_status()
                channels.add(channel)


async def request(client, actor, method, suffix, **kwargs):
    return await client.request(
        method, actor.url(suffix), headers=actor.headers, **kwargs
    )


async def authenticated_request(client, actor, method, url, **kwargs):
    extra = kwargs.pop("headers", {})
    initial = actor.headers.copy()
    response = await client.request(method, url, headers={**initial, **extra}, **kwargs)
    if (
        response.status_code != 401
        or actor.path != "console"
        or not any(k.lower() == "cookie" for k in initial)
    ):
        return response
    async with actor.refresh_lock:
        if actor.headers == initial:
            refreshed = await client.post(
                actor.url("auth/refresh"), headers=actor.headers
            )
            if not refreshed.is_success:
                return response
            jar = {
                part.strip().split("=", 1)[0]: part.strip().split("=", 1)[1]
                for part in actor.headers.get("Cookie", "").split(";")
                if "=" in part
            }
            jar.update(dict(refreshed.cookies))
            actor.headers["Cookie"] = "; ".join(f"{k}={v}" for k, v in jar.items())
            actor.refreshes += 1
    return await client.request(
        method, url, headers={**actor.headers, **extra}, **kwargs
    )


async def verified_identity(client, actor):
    if actor.path == "console":
        response = await authenticated_request(
            client, actor, "GET", actor.url("auth/me")
        )
    elif actor.path == "work-chat":
        response = await client.get(
            actor.url("virtual-users/me"), headers=actor.headers
        )
    else:
        response = await client.post(actor.url("runtime-auth"), headers=actor.headers)
    response.raise_for_status()
    body = response.json()
    identity = (
        body.get("virtual_user_id")
        if actor.path == "agent-api"
        else body.get("id", body.get("user", {}).get("id"))
    )
    if not isinstance(identity, str) or not identity:
        raise ValueError("target did not prove a distinct authenticated principal")
    return hashlib.sha256(identity.encode()).hexdigest()
