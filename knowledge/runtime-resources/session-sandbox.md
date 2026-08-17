---
type: Specification
title: "Session Sandbox"
description: "Managed session-owned sandbox capability and lifecycle."
tags:
  - everruns
  - runtime-resources
---
# Session Sandbox

Managed, session-owned sandbox capability and lifecycle orchestration.

## Goal

Provide one sandbox per session with a provider-neutral tool surface and
server-managed lifecycle:

- auto-start on session creation
- pause after session idle timeout
- resume on next sandbox tool use
- replacement and workspace restore when the physical provider sandbox is lost
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
      "workspace_path": "/home/daytona/workspace",
      "recovery": {
        "enabled": true,
        "volume_name": "everruns-recovery",
        "retained_revisions": 10
      }
    },
    "init": {
      "commands": ["echo ready"]
    }
  }
}
```

### Config contract

See `crates/platform/src/session_sandbox.rs` for the full type definitions.

- `provider`: required provider id
- `auto_start`: best-effort sandbox start on session creation
- `idle_pause_after_seconds`: delay before auto-pause after `session.idled`
- `provider_config`: provider-specific non-secret config
- `init.commands`: one-time commands executed after first successful create

Daytona recovery configuration also accepts `volume_id` to use a
pre-provisioned volume and `mount_path` to override
`/mnt/everruns-recovery`. When `volume_id` is absent, Everruns gets or creates
the configured shared volume by name.

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

### Platform

`crates/platform/src/session_sandbox.rs` defines:

- config, state, and response types
- `SessionSandboxProvider` trait
- provider plugin registration via `inventory`
- state persistence helpers using session secret storage
- generic create/resume/pause/delete/init/checkpoint helpers

`crates/platform/src/capabilities/session_sandbox.rs` exposes the capability and
generic tools. Tool execution resolves the configured provider and delegates
through the trait. After a completed shell or file mutation, the provider
checkpoint is persisted before the tool result is returned to the runtime.

### Integrations

Provider implementations live in integration crates and register with:

`everruns_platform::SessionSandboxProviderPlugin`

Daytona is the first implementation and lives in:

`integrations/daytona/src/session_sandbox_provider.rs`

### Daytona recovery

The coding harness keeps the live worktree on Daytona's writable local filesystem at
`/home/daytona/workspace`. A Daytona Volume is mounted separately at
`/mnt/everruns-recovery`; it is a recovery journal, not the live worktree.

Everruns uses one shared volume per Daytona connection and isolates each
logical sandbox with `sessions/<session_id>` as the Daytona volume subpath.
The stable binding is stored in provider state:

- volume id and name
- mount path and session subpath
- retained revision count
- authoritative completed revision

Each completed mutating operation creates an immutable `tar.gz` workspace
revision plus SHA-256 checksum and completion marker. Rebuildable caches
(`node_modules`, `target`, `.venv`, and `.cache`) are excluded. The default
retention is the newest ten revisions.

If Daytona returns `404` for the physical sandbox, its observed status becomes
`lost`. The next operation creates a new Daytona sandbox with the persisted
volume id and subpath, verifies and restores the persisted revision into the
local worktree, updates the disposable provider id, and continues the same
session. Processes, memory, and interrupted commands are not restored.

Explicit logical deletion clears the isolated recovery subpath before deleting
the physical sandbox. If the physical sandbox was already lost, Everruns mounts
the subpath on a temporary replacement, clears it, and then deletes that
replacement.

The persisted revision pointer, rather than the volume's convenience `HEAD`
file, selects recovery state. This prevents a completely written but
unpersisted revision from being selected after control-plane failure.

The first-class `sandboxes` and `sandbox_checkpoints` records the sandbox
abstraction describes now exist (migration 121, EVE-870). Each upload is
recorded as an unattached checkpoint before the pointer is written and attached
after, so a crash mid-sequence leaves a collectable orphan rather than an
authoritative pointer to a revision no committed turn produced. Attaching is
fenced on the sandbox generation, so a replaced sandbox cannot have its pointer
advanced by an in-flight upload.

Recovery still reads the revision from the encrypted `session_sandbox` secret,
so the checkpoint records are observational for now and state persistence is
still not in the same database transaction as the durable tool-result/event
commit. Closing that window is the remaining EVE-870 work: reconcile the head
checkpoint against `durable_tool_results` at resume, and move recovery onto
`sandboxes.current_checkpoint_id`.

### Server lifecycle

Server-side orchestration lives in:

`crates/server/src/domains/session_sandbox/service.rs`

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
secret name `session_sandbox`. The logical recovery binding lives there; the
Daytona sandbox id is an incarnation that may change after recovery.

Provider-owned remote resources should also register leased resources so the
session resource registry and cleanup infrastructure can see them. Daytona does
this through the existing leased-resource store.

## Non-goals

- multiple managed sandboxes per session
- public/documentation-site docs before the feature stabilizes
- durable distributed idle scheduling in the first experimental version
