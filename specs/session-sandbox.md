# Session Sandbox

Managed, session-owned sandbox capability and lifecycle orchestration.

## Goal

Provide one sandbox per session with a provider-neutral tool surface and
server-managed lifecycle:

- auto-start on session creation
- pause after session idle timeout
- resume on next sandbox tool use
- optional one-time init commands
- provider pluggability (Daytona first)

This feature is experimental and gated by the backend-only
`FEATURE_SESSION_SANDBOX` flag.

## Capability

Capability id: `session_sandbox`

Configured through normal capability config on harness, agent, or session:

```json
{
  "ref": "session_sandbox",
  "config": {
    "provider": "daytona",
    "auto_start": true,
    "idle_pause_after_seconds": 180,
    "provider_config": {
      "size": "small",
      "workspace_path": "/home/daytona"
    },
    "init": {
      "commands": ["echo ready"]
    }
  }
}
```

### Config contract

See `crates/core/src/session_sandbox.rs` for the full type definitions.

- `provider`: required provider id
- `auto_start`: best-effort sandbox start on session creation
- `idle_pause_after_seconds`: delay before auto-pause after `session.idled`
- `provider_config`: provider-specific non-secret config
- `init.commands`: one-time commands executed after first successful create

## Tool surface

Provider-neutral tools exposed by the capability:

- `sandbox_exec`
- `sandbox_read_file`
- `sandbox_write_file`
- `sandbox_status`
- `sandbox_manage`

`session_sandbox` intentionally does not expose `sandbox_create`/`sandbox_list`.
The session owns exactly one sandbox, and provider selection comes from config.

## Architecture

### Core

`crates/core/src/session_sandbox.rs` defines:

- config, state, and response types
- `SessionSandboxProvider` trait
- provider plugin registration via `inventory`
- state persistence helpers using session secret storage
- generic create/resume/pause/delete/init helpers

`crates/core/src/capabilities/session_sandbox.rs` exposes the capability and
generic tools. Tool execution resolves the configured provider and delegates
through the trait.

### Integrations

Provider implementations live in integration crates and register with:

`everruns_core::SessionSandboxProviderPlugin`

Daytona is the first implementation and lives in:

`integrations/daytona/src/session_sandbox_provider.rs`

### Server lifecycle

Server-side orchestration lives in:

`crates/server/src/services/session_sandbox.rs`

Responsibilities:

- resolve effective `session_sandbox` config from harness + agent + session
- best-effort auto-start after session creation
- cancel idle pause on `session.activated`
- schedule idle pause on `session.idled`

Current implementation uses in-process timers. This is acceptable for the
experimental flag. Durable multi-instance scheduling can replace it later if
the feature graduates.

## State and cleanup

Managed sandbox state is stored in encrypted session secret storage under the
secret name `session_sandbox`.

Provider-owned remote resources should also register leased resources so the
session resource registry and cleanup infrastructure can see them. Daytona does
this through the existing leased-resource store.

## Non-goals

- multiple managed sandboxes per session
- public/documentation-site docs before the feature stabilizes
- durable distributed idle scheduling in the first experimental version
