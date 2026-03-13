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
import os
import random
import string

from everruns_sdk import Client


async def main() -> None:
    client = Client(api_key=os.environ["EVERRUNS_API_KEY"], base_url=os.environ["EVERRUNS_BASE_URL"])

    suffix = "".join(random.choices(string.ascii_lowercase + string.digits, k=8))

    agent = await client.agents.create(
        name=f"sdk-compat-py-{suffix}",
        system_prompt="Compatibility test agent",
    )
    fetched_agent = await client.agents.get(agent.id)
    if fetched_agent.id != agent.id:
        raise RuntimeError(f"agent id mismatch: {fetched_agent.id} != {agent.id}")

    session = await client.sessions.create(agent_id=agent.id)
    fetched_session = await client.sessions.get(session.id)
    if fetched_session.id != session.id:
        raise RuntimeError(f"session id mismatch: {fetched_session.id} != {session.id}")

    print(f"ok python sdk {os.environ['SDK_VERSION']}")


asyncio.run(main())
PY
