#!/usr/bin/env bash
set -euo pipefail

version="${1:?usage: test-python.sh <version> <base_url> <api_key>}"
base_url="${2:?usage: test-python.sh <version> <base_url> <api_key>}"
api_key="${3:?usage: test-python.sh <version> <base_url> <api_key>}"

workdir="$(mktemp -d)"
trap 'rm -rf "$workdir"' EXIT

python3 -m venv "$workdir/.venv"
source "$workdir/.venv/bin/activate"

pip install --quiet "everruns-sdk==$version"

# Resolve the Generic harness by name — UUIDs are assigned at DB seed time
# and are no longer stable across deployments. Built-in harness seeding
# happens after server startup, so wait for it explicitly instead of assuming
# /health implies the harness is already queryable.
harness_id="$("$(dirname "$0")/resolve-harness-id.sh" "$base_url" "$api_key")"

EVERRUNS_BASE_URL="$base_url" EVERRUNS_API_KEY="$api_key" EVERRUNS_HARNESS_ID="$harness_id" SDK_VERSION="$version" python3 - <<'PY'
import asyncio
import inspect
import os
import random
import string
import json
import urllib.request

from everruns_sdk import Everruns


def api_post(path: str, body: dict) -> dict:
    request = urllib.request.Request(
        f"{os.environ['EVERRUNS_BASE_URL'].rstrip('/')}{path}",
        data=json.dumps(body).encode(),
        headers={
            "Authorization": f"Bearer {os.environ['EVERRUNS_API_KEY']}",
            "Content-Type": "application/json",
        },
        method="POST",
    )
    with urllib.request.urlopen(request) as response:
        return json.load(response)


async def main() -> None:
    client = Everruns(
        api_key=os.environ["EVERRUNS_API_KEY"],
        base_url=os.environ["EVERRUNS_BASE_URL"],
    )
    version = os.environ["SDK_VERSION"]
    suffix = "".join(random.choices(string.ascii_lowercase + string.digits, k=8))

    # 1. Create agent
    agent = await client.agents.create(
        name=f"sdk-compat-py-{suffix}",
        system_prompt="Compatibility test agent",
    )
    print(f"  agent created: {agent.id}")

    # 2. Fetch agent and verify round-trip
    fetched_agent = await client.agents.get(agent.id)
    if fetched_agent.id != agent.id:
        raise RuntimeError(f"agent id mismatch: {fetched_agent.id} != {agent.id}")
    print("  agent fetch verified")

    # 3. Create session — API changed between SDK versions:
    #    v0.1.0-v0.1.2: sessions.create(agent_id)
    #    v0.1.3+: sessions.create(harness_id, agent_id=...)
    sig = inspect.signature(client.sessions.create)
    params = list(sig.parameters.keys())

    if "harness_id" in params:
        # Generic harness ID resolved by name at runtime
        harness_id = os.environ["EVERRUNS_HARNESS_ID"]
        session = await client.sessions.create(harness_id, agent_id=agent.id)
    else:
        session = await client.sessions.create(agent_id=agent.id)
    print(f"  session created: {session.id}")

    # 4. Fetch session and verify
    fetched_session = await client.sessions.get(session.id)
    if fetched_session.id != session.id:
        raise RuntimeError(f"session id mismatch: {fetched_session.id} != {session.id}")
    print("  session fetch verified")

    # 5. Exercise newly generated trigger and participant API surfaces. Raw
    # HTTP keeps older published SDK cohorts compatible with the current API.
    trigger = api_post(
        f"/v1/agents/{agent.id}/triggers",
        {
            "cron_expression": "0 0 * * *",
            "timezone": "UTC",
            "session_mode": "session_per_invocation",
            "message": "SDK compatibility trigger",
            "enabled": False,
        },
    )
    if trigger.get("agent_id") != agent.id:
        raise RuntimeError("created trigger has wrong agent")
    print(f"  trigger created: {trigger['id']}")

    guest = await client.agents.create(
        name=f"sdk-compat-py-guest-{suffix}",
        system_prompt="Compatibility guest agent",
    )
    participant = api_post(
        f"/v1/sessions/{session.id}/participants",
        {"kind": "agent", "agent_id": guest.id},
    )
    if participant.get("agent_id") != guest.id:
        raise RuntimeError("created participant has wrong agent")
    print(f"  participant added: {participant['id']}")

    print(f"ok python sdk {version}")


asyncio.run(main())
PY
