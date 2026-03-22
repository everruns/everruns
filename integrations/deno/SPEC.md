# Deno Sandbox Capability Specification

## Abstract

The Deno capability integrates [Deno Sandboxes](https://docs.deno.com/sandbox/) as a cloud execution environment for agents. Agents can create, execute commands in, read from, write to, list, and delete multiple isolated sandboxes per session.

**Status**: Available (all environments)

## Architecture

### Transport model

Deno sandboxes use two APIs:

1. **Console API** (`https://console.deno.com/api/v2`) — list sandboxes and validate tokens.
2. **Sandbox API** (`wss://<region>.sandbox-api.deno.net/api/v3/...`) — create/connect sandboxes and run JSON-RPC filesystem/process operations over websocket.

Everruns opens a fresh websocket per tool call. This keeps tool execution stateless between calls while preserving sandbox state remotely.

### State management

Per-sandbox state is stored in session secrets (encrypted at rest):

- secret name: `deno_sandbox:{sandbox_id}`
- secret value: JSON-serialized `SandboxState`

`SandboxState` stores only non-secret metadata: sandbox ID, region, optional org slug, workspace path, and start time.

### Credential resolution

Credential resolution order:

1. User connection for provider `deno`
2. Environment fallback: `DENO_DEPLOY_TOKEN` and optional `DENO_DEPLOY_ORG`

The env fallback exists for operator-managed deployments and live smoke tests.

## Tool surface

### `deno_create_sandbox`

Create a new sandbox.

Parameters:
- `title?` — label shown in Deno metadata
- `region?` — region like `ord` or `ams`
- `timeout?` — concrete lifetime like `20m` or `600s`
- `memory_mb?` — memory in MiB
- `allow_net?` — outbound allowlist entries

Returns: `{ sandbox_id, region, workspace_path, status, timeout }`

### `deno_exec`

Run a shell command with `bash -lc` inside a sandbox.

Parameters:
- `sandbox_id`
- `command`
- `cwd?`

Returns: `{ exit_code, stdout, stderr, output }`

### `deno_read_file`

Read a text file from the sandbox.

Parameters: `sandbox_id`, `path`

Returns: `{ path, content }`

### `deno_write_file`

Write a text file into the sandbox.

Parameters: `sandbox_id`, `path`, `content`

Returns: `{ path, success }`

### `deno_list_sandboxes`

List session-owned sandboxes from persisted session state.

Returns: `{ sandboxes, count }`

### `deno_manage_sandbox`

Delete a sandbox.

Parameters: `sandbox_id`, `action="delete"`

Returns: `{ sandbox_id, action, success }`

## Design decisions

### Concrete timeout required

Deno's default sandbox lifetime is `session`, meaning the sandbox dies when the creator websocket disconnects. Everruns closes the creator websocket after each tool call, so sandboxes must be created with a concrete timeout (for example `20m`). `deno_create_sandbox` rejects `timeout="session"`.

### Session-owned listing

Like Daytona, Everruns lists only sandboxes created in the current session by reading session secret state. This avoids cross-session leakage and does not depend on remote label filtering for correctness.

### Leased-resource cleanup

Each created Deno sandbox registers a leased resource with provider `deno` and type `sandbox`. Cleanup deletes the sandbox remotely, then removes the session secret. The lease metadata stores the non-secret region and optional org slug so cleanup can reconnect to the correct Deno control plane without depending on unrelated process env state.

## Security

Reviewed categories:
- `TM-TOOL` — new tool registration and websocket RPC execution path
- `TM-AGENT-019` / high-risk execution — remote compute with network access
- `TM-DOS` — fixed timeout prevents unbounded sandbox lifetime from idle sessions

Threat summary:
- Sandboxes have network access by design; capability is high-risk and Admin-gated.
- Tokens are resolved from user connections or operator env vars and never stored in sandbox state.
- Session isolation relies on per-session secret names and leased-resource ownership.

See `specs/threat-model.md` for the Deno-specific threat section.
