# Cursor Cloud Agents Integration

## Purpose

Everruns integrates with Cursor Cloud Agents so an Everruns agent can triage work, start one or more asynchronous Cursor agents, monitor their status, send follow-ups, and summarize their results.

Primary use case: a user configures an orchestration agent that investigates an issue, decomposes the work, then launches Cursor Cloud Agents against GitHub repositories for implementation.

## Architecture

- Crate: `integrations/cursor`
- Capability id: `cursor`
- Connection provider id: `cursor`
- API base: `https://api.cursor.com`
- Credential sources:
  1. User connection token from Settings > Connections
  2. Operator env var `CURSOR_API_KEY` for Doppler-backed deployments and live tests
  3. Session secret `CURSOR_API_KEY` as a compatibility fallback

The crate uses Cursor's public Background/Cloud Agents REST API directly. Cursor's public docs do not publish an official Rust SDK, so the integration keeps a small typed `reqwest` client in `src/client.rs`.

## Tool Surface

| Tool | Purpose | Mutates Cursor State |
|---|---|---|
| `cursor_launch_agent` | Start a Cloud Agent for a GitHub repository | Yes |
| `cursor_get_agent` | Read status/result metadata for one agent | No |
| `cursor_list_agents` | Page through existing agents | No |
| `cursor_add_followup` | Send extra instructions to a running agent | Yes |
| `cursor_get_conversation` | Read agent conversation transcript | No |
| `cursor_delete_agent` | Delete a Cursor agent record/resources | Yes, destructive |
| `cursor_list_models` | List recommended model ids | No |
| `cursor_list_repositories` | List GitHub repositories Cursor can access | No, heavily rate-limited |
| `cursor_key_info` | Validate/report active API key metadata | No |

The tool surface intentionally omits webhook secret and image prompt fields for the first release. Both require more careful secret/binary handling so sensitive values and large payloads are not stored in ordinary tool-call history.

## Connection UX

`CursorConnectionProvider` renders an API-key form and validates credentials with `GET /v0/me`. Validation does not launch agents or call repository listing.

The form instructions point users to Cursor Dashboard > Cloud Agents > My Settings > API Keys. Cursor Cloud Agents require a Cloud Agents API key; general Cursor dashboard keys may not authorize agent creation.

## Seed Agent

`Cursor Agent Manager` is registered in `crates/server/src/seed.rs`. It includes:

- `cursor`
- `stateless_todo_list`
- `session_file_system`

The seed prompt teaches the agent to triage, launch scoped Cursor agents, monitor progress, send follow-ups, and avoid repeated repository-list calls because Cursor rate-limits that endpoint.

## State Management

The integration is stateless inside Everruns. Cursor owns agent lifecycle state and returns agent ids, target branches, PR URLs, summaries, and conversation transcripts.

Everruns does not claim ownership of remote Cursor agents per session because Cursor's API lists agents account-wide and agents may outlive an Everruns session. Tool outputs include the Cursor `agent_id`; follow-up/status/delete calls require that id.

## Security

Threat model entries live in `specs/threat-model.md` under `TM-CURSOR`.

Key controls:

- Prefer user connections and encrypted-at-rest credentials.
- Return `ConnectionRequired` when no token exists instead of asking for keys in chat.
- Mark launch/follow-up/delete tools as external, secret-backed, and mutating/destructive as appropriate.
- Bound prompt/id/ref/model/branch input lengths before calling Cursor.
- Do not log API keys.
- Do not support webhook secrets until a dedicated secret flow exists.

Residual risks:

- Cursor agents run in Cursor-managed remote environments with internet access and can execute commands; repository and secret exposure depends on Cursor/GitHub settings.
- Cursor GitHub app access is controlled outside Everruns.
- Account-wide Cursor API keys can create agents up to plan limits; operators/users must scope and rotate keys.

## Tests

- Unit/client tests: `cargo test -p everruns-integrations-cursor`
- Tool integration tests: `tests/tool_integration.rs` uses wiremock plus mock connection resolution.
- Registration tests: inventory plugin and connection provider checks.
- Live API tests: `tests/live_api_test.rs`, gated by `cursor-live-tests`, fail closed if `CURSOR_API_KEY` is missing.

Live test command:

```bash
doppler run -- cargo test -p everruns-integrations-cursor --features cursor-live-tests --test live_api_test -- --test-threads=1
```

Live tests validate `GET /v0/me`, `GET /v0/models`, and a non-creating `GET /v0/agents?limit=1` smoke path. They intentionally do not launch a Cloud Agent to avoid remote write/cost side effects in routine checks.

## CI

- Unit coverage is included in `ci.yml` and `.github/workflows/cursor-integration.yml`.
- Live coverage runs on push to `main`, workflow dispatch, and the weekly integration live sweep via Doppler-provided `CURSOR_API_KEY`.
