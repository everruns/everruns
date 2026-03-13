#!/usr/bin/env bash
set -euo pipefail

version="${1:?usage: test-typescript.sh <version> <base_url> <api_key>}"
base_url="${2:?usage: test-typescript.sh <version> <base_url> <api_key>}"
api_key="${3:?usage: test-typescript.sh <version> <base_url> <api_key>}"

workdir="$(mktemp -d)"
trap 'rm -rf "$workdir"' EXIT

cat > "$workdir/package.json" <<JSON
{
  "name": "sdk-compat-ts",
  "private": true,
  "type": "module",
  "dependencies": {
    "@everruns/sdk": "$version"
  }
}
JSON

cat > "$workdir/test.mjs" <<'JS'
import { Everruns } from "@everruns/sdk";

const client = new Everruns({
  apiKey: process.env.EVERRUNS_API_KEY,
  baseUrl: process.env.EVERRUNS_BASE_URL,
});

const suffix = Math.random().toString(36).slice(2, 10);

// 1. Create agent
const agent = await client.agents.create({
  name: `sdk-compat-ts-${suffix}`,
  systemPrompt: "Compatibility test agent",
});
console.log(`  agent created: ${agent.id}`);

// 2. Fetch agent and verify round-trip
const fetchedAgent = await client.agents.get(agent.id);
if (fetchedAgent.id !== agent.id) {
  throw new Error(`agent id mismatch: ${fetchedAgent.id} != ${agent.id}`);
}
console.log("  agent fetch verified");

// 3. Create session
// v0.1.0 only sends agent_id; server auto-generates harness_id.
// Newer versions may also pass harnessId.
const harnessId = `harness_${[...Array(32)]
  .map(() => Math.floor(Math.random() * 16).toString(16))
  .join("")}`;
const session = await client.sessions.create({ harnessId, agentId: agent.id });
console.log(`  session created: ${session.id}`);

// 4. Fetch session and verify
const fetchedSession = await client.sessions.get(session.id);
if (fetchedSession.id !== session.id) {
  throw new Error(`session id mismatch: ${fetchedSession.id} != ${session.id}`);
}
console.log("  session fetch verified");

console.log(`ok typescript sdk ${process.env.SDK_VERSION}`);
JS

(
  cd "$workdir"
  npm install --silent 2>&1
  EVERRUNS_API_KEY="$api_key" \
  EVERRUNS_BASE_URL="$base_url" \
  SDK_VERSION="$version" \
  node test.mjs
)
