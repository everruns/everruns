# Daytona Capability Specification

## Abstract

The Daytona capability integrates [Daytona](https://www.daytona.io/) cloud-based sandboxes as an agent execution environment. Agents can create, manage, and interact with multiple isolated environments per session via the Daytona REST API. Supports multiple sandboxes per session, each identified by a `sandbox_id`.

**Status**: Available (All environments)

## Architecture

### Two-Tier API

Daytona exposes two API layers:

1. **Management API** (`https://app.daytona.io/api`) — sandbox lifecycle (create, start, stop, delete). Auth: `Bearer <api_key>`.
2. **Toolbox API** (`https://proxy.app.daytona.io/toolbox/{sandboxId}`) — in-sandbox operations (exec, files). Auth: `Bearer <api_key>`.

```
┌────────────────────────────────────────────┐
│              Agent Session                  │
│                                             │
│  Tool Call (daytona_exec, etc.)             │
│         ↓                                   │
│  Load SandboxState from session KV store   │
│         ↓                                   │
│  ┌─────────────────────────────────────┐   │
│  │ DaytonaClient                       │   │
│  │  - Management API (lifecycle)       │   │
│  │  - Toolbox API (exec/files)         │   │
│  └─────────────────────────────────────┘   │
│         ↓                                   │
│  Return result to agent                    │
└────────────────────────────────────────────┘
```

### State Management

Per-sandbox state is stored in session **secrets** (encrypted at rest via AES-256-GCM envelope encryption):

- **Per-sandbox state**: secret name `daytona_sandbox:{sandbox_id}`, value is JSON-serialized `SandboxState`

### API Key Resolution

The Daytona API key is resolved via **user connection** for the `daytona` provider (Settings > Connections).
If not configured, a `ToolError` guides the user to set up in Settings.

### User Connection

Daytona registers as a `ConnectionProviderPlugin` (API-key type). Users configure their key in **Settings > Connections > Daytona**:

1. User enters API key (from [Daytona Dashboard](https://app.daytona.io) > API Keys)
2. Key validated via `GET /sandbox` endpoint
3. Key encrypted and stored in `user_connections` table

This avoids entering the key in chat (see TM-AGENT-016).

## Data Model

### SandboxState (persisted per sandbox)

| Field | Type | Description |
|-------|------|-------------|
| `sandbox_id` | String | Daytona sandbox ID |
| `workspace_path` | String | Default workspace path in sandbox |
| `started_at` | String (ISO 8601) | When the sandbox was started |

### API Response Types

| Type | Fields | Source |
|------|--------|--------|
| `SandboxInfo` | `id`, `name?`, `state` | `POST /sandbox`, `GET /sandbox/{id}` |
| `ExecResult` | `result`, `exit_code` | `POST /toolbox/{id}/process/execute` |

## API Integration

### Management API Endpoints

| Method | Path | Purpose | Request Body |
|--------|------|---------|-------------|
| POST | `/sandbox` | Create sandbox | `{ name?, image?, resources?: { cpu?, memory?, disk? }, autoStopInterval, autoArchiveInterval, autoDeleteInterval, labels? }` |
| GET | `/sandbox/{id}` | Get sandbox info | — |
| POST | `/sandbox/{id}/start` | Start sandbox | — |
| POST | `/sandbox/{id}/stop` | Stop sandbox | — |
| DELETE | `/sandbox/{id}` | Delete sandbox | — |
| POST | `/sandbox/{id}/autostop/{minutes}` | Set auto-stop | — |

### Toolbox API Endpoints (at `proxy.app.daytona.io/toolbox/{sandboxId}`)

| Method | Path | Purpose | Request Body |
|--------|------|---------|-------------|
| POST | `/process/execute` | Execute command (sync) | `{ command, cwd?, timeout? }` |
| GET | `/files?path=` | List directory | — |
| GET | `/files/download?path=` | Download file | — |
| POST | `/files/upload?path=` | Upload file (multipart) | multipart/form-data |
| DELETE | `/files?path=` | Delete file/directory | — |
| POST | `/files/folder?path=&mode=` | Create directory | — |

## Tools

### daytona_create_sandbox

Creates a new sandbox. Optionally uploads files from session storage.

- **Parameters**:
  - `title`: string (optional) — sandbox name
  - `image`: string (optional) — container image
  - `cpu`: integer (optional) — vCPUs, 1-4 (default: 1)
  - `memory`: integer (optional) — GiB RAM, 1-8 (default: 1)
  - `disk`: integer (optional) — GiB disk, 1-10 (default: 3)
  - `upload_files`: array (optional) — `[{session_path, sandbox_path}]`
- **Returns**: `{ sandbox_id, status, workspace_path }`
- **Resource mapping**: When any of `cpu`/`memory`/`disk` are specified, they are sent as a `resources` object in the Daytona API request body. Only specified fields are included; omitted fields use Daytona defaults. Values are validated client-side (type + range) before the API call.

### daytona_exec

Executes a shell command in a sandbox (synchronous).

- **Parameters**:
  - `sandbox_id`: string (required)
  - `command`: string (required) — shell command
  - `cwd`: string (optional) — working directory
  - `timeout`: integer (optional) — timeout in ms (default: 120000)
- **Returns**: `{ exit_code, output }`

### daytona_read_file

Read a file from sandbox filesystem.

- **Parameters**: `sandbox_id` (required), `path` (required)
- **Returns**: `{ path, content }`

### daytona_write_file

Write content to a file in sandbox.

- **Parameters**: `sandbox_id` (required), `path` (required), `content` (required)
- **Returns**: `{ path, success }`

### daytona_download_workspace

Downloads sandbox workspace to session file storage.

- **Parameters**:
  - `sandbox_id`: string (required)
  - `sandbox_path`: string (optional, default: workspace_path)
  - `session_path`: string (optional, default: `/workspace`)
- **Returns**: `{ files_downloaded, files_skipped, errors? }`

### daytona_list_sandboxes

Lists all sandboxes created in this session.

- **Parameters**: none
- **Returns**: `{ sandboxes: [{sandbox_id, started_at, workspace_path}], count }`

### daytona_manage_sandbox

Lifecycle management: stop or delete.

- **Parameters**:
  - `sandbox_id`: string (required)
  - `action`: `"stop" | "delete"` (required)
- **Returns**: `{ sandbox_id, action, success }`

### daytona_git_clone

Clone a git repository into a sandbox. Automatically uses the user's connected GitHub credentials for private repos.

- **Parameters**:
  - `sandbox_id`: string (required)
  - `repo_url`: string (required) — supports `https://`, `git@`, or `user/repo` shorthand
  - `branch`: string (optional) — branch to clone (defaults to default branch)
  - `path`: string (optional) — destination inside sandbox (defaults to `/home/daytona/<owner>/<repo>`)
- **Returns**: `{ sandbox_id, repo_url, path, branch, commit, authenticated }`

**Implementation:** Runs `git clone` via the `exec` endpoint (`POST /process/execute`). For authenticated clones, the GitHub token is embedded in the HTTPS URL (`https://oauth2:<token>@github.com/...`). Uses `--depth 1` for faster clones.

**Authentication flow:**
1. Lazily resolves GitHub token from user connections (`connection_resolver`)
2. Falls back to `GITHUB_TOKEN` session secret
3. If token found: embeds `oauth2:<token>` in the clone URL
4. If no token: public repos only; private repos fail with hint to connect GitHub

### daytona_git_credentials

Configure git credentials in a sandbox so that all git operations (push, pull, fetch, rebase, etc.) work transparently via `daytona_exec`. Call once after creating a sandbox; call again to refresh if the token expires (~1 hour).

- **Parameters**:
  - `sandbox_id`: string (required)
- **Returns**: `{ sandbox_id, authenticated, provider, hint }`

**Implementation:** Writes a `git credential store` file (`/tmp/.git-credentials`) containing `https://oauth2:<token>@github.com`, then configures `git config --global credential.helper 'store --file=/tmp/.git-credentials'`. After this, any git command via `daytona_exec` authenticates automatically.

**Authentication flow:** Same token resolution as `daytona_git_clone` (connection_resolver → GITHUB_TOKEN fallback). Fails with actionable error if no credentials found.

**Design:** Avoids per-verb tools (git_push, git_fetch, etc.). Instead, configures standard git credential store once, then agent uses `daytona_exec` for all git operations naturally. Token in `/tmp` — lost on sandbox stop, same trust boundary as sandbox exec access.

## Security

- **API Key**: Stored in user connections (Settings > Connections > Daytona), encrypted at rest (TM-DAYTONA-004)
- **Single auth token**: Both Management and Toolbox APIs use the same Bearer token
- **Sandbox Isolation**: Each sandbox is an isolated environment (TM-DAYTONA-005)
- **Multi-tenancy**: Sandboxes scoped to session via secret name prefixes
- **Git credentials**: Short-lived GitHub token written to `/tmp/.git-credentials`; lost on sandbox stop; same trust boundary as exec access (TM-DAYTONA-001)
- **Token expiry**: GitHub App installation tokens expire in ~1 hour; agent must call `daytona_git_credentials` again to refresh (TM-DAYTONA-002)

See [threat-model.md](threat-model.md#16-daytona-cloud-sandbox-tm-daytona) for full threat analysis.

## Error Handling

| Scenario | Result Type | Message |
|----------|-------------|---------|
| Missing required param | `ToolError` | "Missing required parameter: {name}" |
| Sandbox not found | `ToolError` | "Sandbox '{id}' not found. Create one first with daytona_create_sandbox." |
| API key not configured | `ToolError` | "Daytona API key not configured." |
| HTTP 4xx | `ToolError` | "Daytona API error ({status}): {body}" |
| No context | `ToolError` | "{tool_name} requires context." |

## Design Decisions

### Sandbox metadata labels (required)

All sandboxes **must** include Daytona `labels` at creation time for audit, dashboard visibility, and orphan cleanup. Labels are `Record<string, string>` key-value pairs stored on the Daytona side.

Required labels:

| Label key | Value | Source |
|-----------|-------|--------|
| `everruns` | `"true"` | Static — identifies Everruns-owned sandboxes |
| `everruns.session_id` | Session ID | `context.session_id` |
| `everruns.harness_id` | Harness ID | `session.harness_id` |
| `everruns.org_id` | Organization ID | `session.organization_id` |
| `everruns.agent_id` | Agent ID (if set) | `session.agent_id` |

This enables:
- Filtering sandboxes by org/agent/session in the Daytona dashboard
- Automated cleanup of orphaned sandboxes via label queries
- Audit trail linking sandboxes back to their Everruns origin

Any new sandbox integration (Daytona or otherwise) must attach equivalent ownership metadata.

### Synchronous exec with lease heartbeat

Daytona's `POST /process/execute` is inherently synchronous. No async polling needed. The `timeout` parameter handles long-running commands.

During exec, a background heartbeat task renews the sandbox lease every 3 minutes (`LEASE_HEARTBEAT_INTERVAL`). This prevents the leased-resource cleanup and Daytona's auto-stop from killing the sandbox during long-running commands (e.g. Rust compilation taking 20+ minutes). The heartbeat is cancelled when the exec completes.

### Configurable auto-stop

`daytona_create_sandbox` accepts an optional `auto_stop_minutes` parameter (1–60, default: 5). Agents running long builds can request a longer inactivity window at sandbox creation time.

### Simpler state model

Daytona uses a single API key for both management and toolbox APIs. No per-sandbox pitcher/preview tokens needed. SandboxState only stores `sandbox_id`, `workspace_path`, and `started_at`.

### File operations via Toolbox API

Files are managed through the Toolbox API proxy (`proxy.app.daytona.io`). Upload uses multipart/form-data, download returns raw bytes.

### Git credentials via credential store file

Git credentials for push/pull/fetch are configured by writing a `git credential store` file (`/tmp/.git-credentials`) inside the sandbox. This is the same mechanism used by CI systems (GitHub Actions, etc.).

**Considered and dismissed: per-verb git tools.** Creating `daytona_git_push`, `daytona_git_fetch`, `daytona_git_pull`, etc. would duplicate `daytona_exec` with credential injection. Doesn't scale — every new git operation needs a new tool.

**Considered and dismissed: magic git detection in `daytona_exec`.** Auto-detecting git commands and injecting credentials transparently. Fragile heuristic, surprising behavior, hard to debug.

**Future improvement: API-proxied credential helper.** Instead of writing a token file, configure git in the sandbox to call back to an Everruns API endpoint (e.g. `GET /api/sessions/{id}/git-credential`) that mints a fresh token on each request. Benefits: no token on disk, always fresh (no expiry), multi-provider (GitHub/GitLab) via query param, per-session ACLs. Deferred because the credential store approach is simpler, works now, and the sandbox is already an isolated trust boundary.

## Crate Structure

`integrations/daytona/` → `everruns-integrations-daytona`

External integration crate, auto-registered via `inventory::submit!` plugin system.

**Force-link required**: Both `crates/server/src/lib.rs` and `crates/worker/src/lib.rs` must contain `extern crate everruns_integrations_daytona;` — otherwise the linker optimizes out the crate and `inventory::submit!` registrations silently disappear. See [architecture.md](architecture.md#integration-plugin-force-linking).

| File | Purpose |
|------|---------|
| `src/lib.rs` | Plugin registration, constants, `DaytonaCapability` impl |
| `src/client.rs` | `DaytonaClient` HTTP client (management + toolbox APIs), URL encoding |
| `src/connection.rs` | `DaytonaConnectionProvider` — API-key connection plugin |
| `src/state.rs` | API types (`SandboxInfo`, `ExecResult`, `SandboxState`), session state helpers |
| `src/tools.rs` | 9 tool implementations (`DaytonaCreateSandboxTool`, etc.) |
| `tests/plugin_registration.rs` | Integration tests for inventory registration |
| `tests/tool_integration.rs` | Integration tests: tool execution + wiremock Daytona API |
| `tests/live_api_test.rs` | Live API integration tests (feature-gated: `daytona-live-tests`) |

## Capability Registration

- **ID**: `daytona`
- **Name**: `Daytona`
- **Status**: Available
- **Icon**: `cloud`
- **Category**: `Execution`
- **Dependencies**: `["session_storage"]`

## Seeded Agent: Daytona Coder

A pre-configured seed agent (`Daytona Coder`) demonstrates the capability:
- **Capabilities**: `daytona`, `session_storage`, `session_file_system`
- **Dev-only**: false
