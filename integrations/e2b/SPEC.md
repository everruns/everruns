# E2B Capability Specification

## Abstract

The E2B capability integrates [E2B](https://e2b.dev/docs) cloud sandboxes as an agent execution environment. Agents can create, pause, resume, delete, and interact with isolated Linux environments per session. Users bring their own E2B API key via the connection provider; no platform-owned or environment-variable fallback is accepted.

**Status**: Available (All environments)

## Architecture

### Split Control/Data Plane

E2B uses two surfaces:

1. **Management API** (`https://api.e2b.app`), sandbox lifecycle, metadata, timeout, and detail fetches. Auth: `X-API-KEY: <key>`.
2. **envd sandbox endpoint** (`https://49983-<sandboxId>.<domain>`), in-sandbox file access and command execution. Auth: `X-Access-Token`, `E2b-Sandbox-Id`, and `E2b-Sandbox-Port` headers.

The Everruns integration mirrors Daytona's session-scoped state model, but uses E2B's native split between the management API and envd runtime endpoints.

## Authentication Model

### BYO API Key

Users connect their own E2B account via the connection provider (API key form). Credential resolution:

1. The E2B tool calls resolve the API key from the session's `e2b` user connection.
2. If no connection is configured, the tool returns `ConnectionRequired`, triggering the inline connection dialog.
3. There is no env-var or platform-wide fallback. Every E2B operation requires a user-provided key.

This keeps credentials out of message history and scopes sandbox costs and quotas to each user's E2B account.

## State Management

Per-sandbox state is stored in session secrets under `e2b_sandbox:{sandbox_id}`. Stored fields:

- `sandbox_id`
- `sandbox_domain`
- `envd_version`
- `envd_access_token`
- `workspace_path`
- `started_at`
- `timeout_seconds`

Leased resources register provider `e2b`, type `sandbox`, enabling generic cleanup on the worker side.

## Tools

### e2b_create_sandbox

Creates a new sandbox from an E2B template.

- **Parameters**:
  - `title` (optional)
  - `template` (optional, default `base`)
  - `timeout_seconds` (optional, default `3600`)
  - `env_vars` (optional object)
  - `upload_files` (optional array of `{session_path, sandbox_path}`)
- **Returns**: `{ sandbox_id, status, template, workspace_path, timeout_seconds, files_uploaded }`

### e2b_exec

Executes a shell command in a sandbox using E2B's process service.

- **Parameters**:
  - `sandbox_id` (required)
  - `command` (required)
  - `cwd` (optional)
  - `timeout_ms` (optional)
- **Returns**: `{ sandbox_id, cwd, stdout, stderr, exit_code, success, truncated, total_lines, error }`
- **Human representation**: Follows the shared exec-tool contract in `knowledge/execution/tool-execution.md`. Command text is primary; sandbox metadata is secondary.

### e2b_read_file

Reads a file from the sandbox filesystem.

- **Parameters**: `sandbox_id`, `path`
- **Returns**: `{ sandbox_id, path, content }`

### e2b_write_file

Writes a file into the sandbox filesystem.

- **Parameters**: `sandbox_id`, `path`, `content`
- **Returns**: `{ sandbox_id, path, success }`

### e2b_list_sandboxes

Lists all E2B sandboxes created in the current session.

- **Parameters**: none
- **Returns**: `{ sandboxes: [...], count }`

### e2b_manage_sandbox

Lifecycle management.

- **Parameters**:
  - `sandbox_id` (required)
  - `action` (`pause | resume | delete`)
  - `timeout_seconds` (optional; used for `resume`)
- **Returns**: `{ sandbox_id, action, success, timeout_seconds? }`

## Metadata

All created sandboxes include Everruns ownership metadata:

- `everruns=true`
- `everruns.title`
- `everruns.session_id`
- `everruns.harness_id`
- `everruns.org_id`
- `everruns.agent_id` (when available)

This supports dashboard traceability, orphan cleanup, and audit review.

## Security

- **API key** resolves from the user connection; never stored in env vars or emitted in tool output.
- **envd access tokens** are session-scoped and stored only in encrypted session secrets.
- **Sandbox isolation** depends on E2B runtime boundaries plus Everruns session-scoped secret lookups.
- **Resource leak mitigation** uses E2B timeout + auto-pause plus Everruns leased-resource cleanup.

## Error Handling

| Scenario | Result Type | Message |
|---|---|---|
| Missing required param | `ToolError` | `Missing required parameter: {name}` |
| No E2B connection | `ConnectionRequired` | triggers inline connection dialog |
| Missing sandbox state | `ToolError` | `Sandbox '{id}' not found...` |
| E2B lifecycle/file/process error | `ToolError` | `E2B API error (...)` or sandbox-specific error |
| Missing storage context | `ToolError` | `Storage not available in this context` |

## Testing

### Unit tests

```bash
cargo test -p everruns-integrations-e2b
```

Covers capability metadata, plugin registration, state serialization, and HTTP request shaping for management/file APIs.

### Live API smoke tests

```bash
E2B_API_KEY=<key> cargo test -p everruns-integrations-e2b --features e2b-live-tests --test live_api_test
```

Exercises sandbox create → file write/read → command exec → cleanup against the real E2B service. The live test reads `E2B_API_KEY` directly from the environment (bypassing the connection provider), so you set the env var when running tests locally. Missing-credential behavior is **fail-closed**: with the feature flag on but `E2B_API_KEY` unset, the test panics (see `knowledge/integrations/integrations.md`).

CI keeps E2B live coverage off `pull_request`: `.github/workflows/ci.yml` runs the live test only on pushes to `main` when `integrations/e2b/**` changes. `.github/workflows/integration-live-sweep.yml` reruns the same live path weekly and on demand to catch shared regressions that path filters miss.
