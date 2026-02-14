# CodeSandbox Capability Specification

## Abstract

The CodeSandbox capability integrates [CodeSandbox](https://codesandbox.io/) cloud-based sandbox VMs as an agent execution environment. Agents can create, manage, and interact with multiple isolated Firecracker microVMs per session via the CodeSandbox REST API. Unlike the Docker Container capability (single container per session), this supports multiple sandboxes per session, each identified by a `sandbox_id`.

**Status**: Experimental (Dev only)

## Architecture

### Two-Tier API

CodeSandbox exposes two API layers:

1. **Management API** (`https://api.codesandbox.io`) — sandbox lifecycle (create, start, shutdown, hibernate, delete). Auth: `Bearer <CSB_API_KEY>`.
2. **Pint API** (dynamic `pitcher_url` per sandbox) — in-sandbox operations (exec, files, directories). Auth: `Bearer <pitcher_token>`. The `pitcher_url` and `pitcher_token` are returned when starting a VM.

```
┌────────────────────────────────────────────┐
│              Agent Session                  │
│                                             │
│  Tool Call (csb_exec, csb_read_file, etc.) │
│         ↓                                   │
│  Load SandboxState from session KV store   │
│         ↓                                   │
│  ┌─────────────────────────────────────┐   │
│  │ CodeSandboxClient                    │   │
│  │  - Management API (lifecycle)        │   │
│  │  - Pint API (exec/files via pitcher) │   │
│  └─────────────────────────────────────┘   │
│         ↓                                   │
│  Return result to agent                    │
└────────────────────────────────────────────┘
```

### State Management

All state is stored in session **secrets** (encrypted at rest via AES-256-GCM envelope encryption):

- **API key**: secret name `CSB_API_KEY`
- **Per-sandbox state**: secret name `csb_sandbox:{sandbox_id}`, value is JSON-serialized `SandboxState`

**Why store connection info?** The `pitcher_url` and `pitcher_token` returned by `POST /vm/{id}/start` are needed for all subsequent Pint API calls. Tools are stateless structs with no shared memory between invocations. Without persistence, every tool call would require an extra HTTP roundtrip (~200ms+) to re-derive connection info. The `pitcher_token` grants full access to the sandbox VM, so encryption at rest is appropriate.

## Data Model

### SandboxState (persisted per sandbox)

| Field | Type | Description |
|-------|------|-------------|
| `sandbox_id` | String | CodeSandbox sandbox ID |
| `pitcher_url` | String | Pint API base URL for this sandbox |
| `pitcher_token` | String | Auth token for Pint API |
| `workspace_path` | String | Default workspace path in sandbox |
| `started_at` | String (ISO 8601) | When the sandbox was started |

### API Response Types

| Type | Fields | Source |
|------|--------|--------|
| `SandboxInfo` | `id`, `title?` | `POST /sandbox` response |
| `VmStartResponse` | `pitcher_url`, `pitcher_token`, `workspace_path?` | `POST /vm/{id}/start` response |
| `ExecInfo` | `id`, `status`, `exit_code?` | `GET /api/v1/execs/{id}` response |
| `FileContent` | `path`, `content` | `GET /api/v1/files/{path}` response |
| `DirEntry` | `name`, `entry_type?` | `GET /api/v1/directories/{path}` response |

## API Integration

### Management API Endpoints

| Method | Path | Purpose | Request Body |
|--------|------|---------|-------------|
| POST | `/sandbox` | Create sandbox | `{ title?, template?, runtime: "vm" }` |
| GET | `/sandbox/{id}` | Get sandbox info | — |
| POST | `/vm/{id}/start` | Start VM | `{ tier? }` |
| POST | `/vm/{id}/shutdown` | Shutdown VM | — |
| POST | `/vm/{id}/hibernate` | Hibernate VM | — |
| DELETE | `/vm/{id}` | Delete VM | — |

### Pint API Endpoints (at `pitcher_url`)

| Method | Path | Purpose | Request Body |
|--------|------|---------|-------------|
| POST | `/api/v1/execs` | Create execution | `{ command: string[] }` |
| GET | `/api/v1/execs/{id}` | Get exec status | — |
| GET | `/api/v1/execs/{id}/io` | SSE output stream | — |
| DELETE | `/api/v1/execs/{id}` | Kill execution | — |
| GET | `/api/v1/files/{path}` | Read file | — |
| POST | `/api/v1/files/{path}` | Create/write file | `{ content? }` |
| DELETE | `/api/v1/files/{path}` | Delete file | — |
| GET | `/api/v1/directories/{path}` | List directory | — |

## Tools

### csb_create_sandbox

Creates a new sandbox VM. Optionally uploads files from session storage.

- **Parameters**:
  - `title`: string (optional) — sandbox title
  - `template`: string (optional) — template ID to fork from
  - `upload_files`: array (optional) — `[{session_path, sandbox_path}]`
- **Returns**: `{ sandbox_id, status, workspace_path }`
- **Policy**: Auto

### csb_exec

Executes a shell command in a sandbox. Supports sync and async modes.

- **Parameters**:
  - `sandbox_id`: string (required)
  - `command`: string (required) — shell command
  - `wait`: boolean (optional, default: true) — wait for completion
- **Returns**: `{ exec_id, status, exit_code?, output? }`
- **Policy**: Auto

### csb_exec_status

Check execution status and get output.

- **Parameters**:
  - `sandbox_id`: string (required)
  - `exec_id`: string (required)
- **Returns**: `{ exec_id, status, exit_code?, output? }`
- **Policy**: Auto

### csb_read_file

Read a file from sandbox filesystem.

- **Parameters**: `sandbox_id` (required), `path` (required)
- **Returns**: `{ path, content }`
- **Policy**: Auto

### csb_write_file

Write content to a file in sandbox.

- **Parameters**: `sandbox_id` (required), `path` (required), `content` (required)
- **Returns**: `{ path, success }`
- **Policy**: Auto

### csb_download_workspace

Downloads entire sandbox workspace to session file storage.

- **Parameters**:
  - `sandbox_id`: string (required)
  - `sandbox_path`: string (optional, default: workspace_path)
  - `session_path`: string (optional, default: `/workspace`)
- **Returns**: `{ files_downloaded, files_skipped, errors? }`
- **Policy**: Auto

### csb_list_sandboxes

Lists all sandboxes created in this session.

- **Parameters**: none
- **Returns**: `{ sandboxes: [{sandbox_id, started_at}] }`
- **Policy**: Auto

### csb_manage_sandbox

Lifecycle management: shutdown, hibernate, or delete.

- **Parameters**:
  - `sandbox_id`: string (required)
  - `action`: `"shutdown" | "hibernate" | "delete"` (required)
- **Returns**: `{ sandbox_id, action, success }`
- **Policy**: Auto

## State Lifecycle

```
  csb_create_sandbox
        │
        ▼
  ┌──────────┐   csb_exec, csb_read_file,    ┌──────────┐
  │ Created   │──csb_write_file, etc.──────── │  Active   │
  │ & Started │◄──────────────────────────────│          │
  └──────────┘                                └──────────┘
        │                                          │
        │    csb_manage_sandbox                    │
        │    action="hibernate"                    │
        ▼                                          ▼
  ┌──────────┐                              ┌──────────┐
  │Hibernated│  (resume via csb_exec auto)  │ Shutdown │
  └──────────┘                              └──────────┘
        │                                          │
        │    csb_manage_sandbox action="delete"    │
        ▼                                          ▼
  ┌──────────┐                              ┌──────────┐
  │ Deleted   │                              │ Deleted   │
  └──────────┘                              └──────────┘
```

## Security

- **API Key**: Stored in session secrets (`CSB_API_KEY`), encrypted at rest via AES-256-GCM envelope encryption
- **Pitcher Token**: Stored in session secrets (`csb_sandbox:{id}`), encrypted at rest. Ephemeral, short-lived, bound to a specific sandbox VM.
- **Sandbox Isolation**: Each sandbox is a Firecracker microVM with complete isolation
- **Multi-tenancy**: Sandboxes scoped to session via secret name prefixes

## Error Handling

| Scenario | Result Type | Message |
|----------|-------------|---------|
| Missing required param | `ToolError` | "Missing required parameter: {name}" |
| Sandbox not found | `ToolError` | "Sandbox '{id}' not found. Create one first with csb_create_sandbox." |
| API key not configured | `ToolError` | "CSB_API_KEY not set. Use secret_store to set it first." |
| HTTP 4xx | `ToolError` | "CodeSandbox API error ({status}): {body}" |
| HTTP 5xx / network | `InternalError` | Logged internally, generic message to LLM |
| No context | `ToolError` | "{tool_name} requires context." |

## Design Decisions

### Multiple sandboxes per session

Unlike Docker Container (single container per session), CodeSandbox supports multiple sandboxes. This enables workflows like running frontend and backend in separate sandboxes, A/B testing different configurations, or parallel execution.

### KV store for connection state

Pitcher URL and token are persisted in session KV store rather than re-derived on each call. This avoids ~200ms+ latency per tool call. The `pitcher_token` is ephemeral (short-lived, sandbox-scoped), so plain-text KV storage is appropriate.

### Sync + async exec modes

The `csb_exec` tool supports both `wait: true` (blocks until completion, returns output) and `wait: false` (returns immediately with exec_id for polling). Sync mode is convenient for simple commands; async mode is essential for long-running tasks.

### Workspace snapshot download

The `csb_download_workspace` tool downloads the entire workspace directory tree rather than individual files. This supports the common workflow of running computations in a sandbox and bringing all results back to session storage.

### session_storage dependency

The capability declares `session_storage` as a dependency because it needs both the secrets store (for `CSB_API_KEY`) and the KV store (for sandbox state persistence).

## Capability Registration

- **ID**: `codesandbox`
- **Name**: `[Experimental] CodeSandbox`
- **Status**: Available (Dev only, gated behind `experimental_features_enabled()`)
- **Icon**: `cloud`
- **Category**: `Execution`
- **Dependencies**: `["session_storage"]`

## Seeded Agent: Cloud Coder

A pre-configured seed agent (`Cloud Coder`) demonstrates the capability:
- **Capabilities**: `codesandbox`, `session_storage`, `session_file_system`
- **Dev-only**: true
- **System prompt**: Guides users through API key setup, sandbox creation, code execution, and result download
