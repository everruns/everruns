# Daytona Capability Specification

## Abstract

The Daytona capability integrates [Daytona](https://www.daytona.io/) cloud-based sandboxes as an agent execution environment. Agents can create, manage, and interact with multiple isolated environments per session via the Daytona REST API. Like CodeSandbox, this supports multiple sandboxes per session, each identified by a `sandbox_id`.

**Status**: Experimental (Dev only)

## Architecture

### Two-Tier API

Daytona exposes two API layers:

1. **Management API** (`https://app.daytona.io/api`) — sandbox lifecycle (create, start, stop, delete). Auth: `Bearer <DAYTONA_API_KEY>`.
2. **Toolbox API** (`https://proxy.app.daytona.io/toolbox/{sandboxId}`) — in-sandbox operations (exec, files). Auth: `Bearer <DAYTONA_API_KEY>`.

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

All state is stored in session **secrets** (encrypted at rest via AES-256-GCM envelope encryption):

- **API key**: secret name `DAYTONA_API_KEY`
- **Per-sandbox state**: secret name `daytona_sandbox:{sandbox_id}`, value is JSON-serialized `SandboxState`

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
| POST | `/sandbox` | Create sandbox | `{ name?, image?, autoStopInterval }` |
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
  - `upload_files`: array (optional) — `[{session_path, sandbox_path}]`
- **Returns**: `{ sandbox_id, status, workspace_path }`

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

## Security

- **API Key**: Stored in session secrets (`DAYTONA_API_KEY`), encrypted at rest
- **Single auth token**: Both Management and Toolbox APIs use the same Bearer token
- **Sandbox Isolation**: Each sandbox is an isolated environment
- **Multi-tenancy**: Sandboxes scoped to session via secret name prefixes

## Error Handling

| Scenario | Result Type | Message |
|----------|-------------|---------|
| Missing required param | `ToolError` | "Missing required parameter: {name}" |
| Sandbox not found | `ToolError` | "Sandbox '{id}' not found. Create one first with daytona_create_sandbox." |
| API key not configured | `ToolError` | "DAYTONA_API_KEY not set." |
| HTTP 4xx | `ToolError` | "Daytona API error ({status}): {body}" |
| No context | `ToolError` | "{tool_name} requires context." |

## Design Decisions

### Shape parity with CodeSandbox

The Daytona integration mirrors the CodeSandbox integration shape: same tool naming pattern (prefix-based), same state management via session secrets, same capability registration via inventory plugin.

### Synchronous exec only

Daytona's `POST /process/execute` is inherently synchronous. No async polling needed (unlike CodeSandbox's exec_create + poll pattern). The `timeout` parameter handles long-running commands.

### Simpler state model

Daytona uses a single API key for both management and toolbox APIs. No per-sandbox pitcher/preview tokens needed. SandboxState only stores `sandbox_id`, `workspace_path`, and `started_at`.

### File operations via Toolbox API

Files are managed through the Toolbox API proxy (`proxy.app.daytona.io`). Upload uses multipart/form-data, download returns raw bytes.

## Crate Location

`integrations/daytona/` → `everruns-integrations-daytona`

External integration crate, auto-registered via `inventory::submit!` plugin system.

## Capability Registration

- **ID**: `daytona`
- **Name**: `[Experimental] Daytona`
- **Status**: Available (Dev only)
- **Icon**: `cloud`
- **Category**: `Execution`
- **Dependencies**: `["session_storage"]`

## Seeded Agent: Daytona Coder

A pre-configured seed agent (`Daytona Coder`) demonstrates the capability:
- **Capabilities**: `daytona`, `session_storage`, `session_file_system`
- **Dev-only**: true
