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

EVERRUNS_BASE_URL="$base_url" EVERRUNS_API_KEY="$api_key" SDK_VERSION="$version" python3 - <<'PY'
import asyncio
import inspect
import os
import random
import string

from everruns_sdk import Everruns


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
        from everruns_sdk import generate_harness_id
        harness_id = generate_harness_id()
        session = await client.sessions.create(harness_id, agent_id=agent.id)
    else:
        session = await client.sessions.create(agent_id=agent.id)
    print(f"  session created: {session.id}")

    # 4. Fetch session and verify
    fetched_session = await client.sessions.get(session.id)
    if fetched_session.id != session.id:
        raise RuntimeError(f"session id mismatch: {fetched_session.id} != {session.id}")
    print("  session fetch verified")

    print(f"ok python sdk {version}")


asyncio.run(main())
PY
