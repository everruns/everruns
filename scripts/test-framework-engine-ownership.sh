#!/usr/bin/env bash
# 0.18 architecture guard: engines own session lifecycle; Agent is behavior.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

cd "$PROJECT_ROOT"

python3 - <<'PY'
from pathlib import Path

agent_path = Path("crates/everruns/src/agent.rs")
agent = agent_path.read_text()
production = agent.split("#[cfg(test)]", 1)[0]

forbidden = {
    "compatibility_engine": "hidden compatibility engine",
    "InMemoryEngine": "concrete engine ownership",
    "issued_sessions": "session identity catalog",
    "pub fn session(": "Agent::session compatibility API",
    "pub async fn resume(": "Agent::resume compatibility API",
}

violations = [label for needle, label in forbidden.items() if needle in production]
if violations:
    joined = ", ".join(violations)
    raise SystemExit(
        f"{agent_path} violates engine-owned session architecture: {joined}"
    )

engine = Path("crates/everruns/src/engine.rs").read_text()
required = (
    "pub struct Engine",
    "pub type InMemoryEngine = Engine",
    "pub fn create(&self, agent: Agent) -> Session",
    "pub async fn resume(&self, session_id: SessionId)",
    "pub async fn attach(&self, session_id: SessionId, agent: Agent)",
)
missing = [needle for needle in required if needle not in engine]
if missing:
    raise SystemExit(
        "crates/everruns/src/engine.rs lost the canonical Engine API: "
        + ", ".join(missing)
    )

facade = Path("crates/everruns/src/lib.rs").read_text()
for forbidden_api in (
    "pub use everruns_core as core",
    "SessionExecution",
    "pub use everruns_host::{\n    HarnessBuilder",
):
    if forbidden_api in facade:
        raise SystemExit(f"facade leaked internal API: {forbidden_api}")
PY

legacy_worker_aliases='GrpcAgentStore|GrpcEventEmitter|GrpcMessageRetriever|GrpcProviderStore|GrpcSessionFileStore|GrpcSessionStore|AdapterAgentStore|AdapterEventEmitter|AdapterMessageRetriever|AdapterProviderStore|AdapterSessionFileStore|AdapterSessionStore'
if rg -n "$legacy_worker_aliases" crates/worker/src crates/server/src </dev/null; then
  echo "worker compatibility adapter aliases must not return." >&2
  exit 1
fi

echo "Framework session lifecycle remains engine-owned; legacy worker aliases are absent."
