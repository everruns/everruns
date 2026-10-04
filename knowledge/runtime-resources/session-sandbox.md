---
type: Specification
title: "Managed Environment Runtime"
description: "Provider-neutral managed Environment capability, workspace durability, and recovery lifecycle."
tags:
  - everruns
  - runtime-resources
---
# Managed Environment Runtime

`session_sandbox` is the internal runtime capability for a managed, session-owned
Environment. Environment profiles are the product configuration surface; the
capability id and legacy `sandbox_*` types remain internal compatibility names.

## Goal

Provide one sandbox per session with a provider-neutral tool surface and
server-managed lifecycle:

- auto-start on session creation
- pause after session idle timeout
- resume on next sandbox tool use
- replacement and workspace restore when the physical provider sandbox is lost
- optional one-time init commands
- provider pluggability (Daytona first)

Managed Environment profiles are available without a separate feature flag.
PostgreSQL deployments must configure `SECRETS_ENCRYPTION_KEY` so lifecycle
state and provider connections can be resolved; the canonical local startup
supplies its stable development key. Daytona execution also requires an
organization or user Daytona connection.

## Capability

Capability id: `session_sandbox`

New sessions receive this configuration from their resolved Agent Environment
profile. Direct capability configuration remains supported for existing data:

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

See `crates/capabilities/src/session_sandbox.rs` for the full type definitions.

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

Stable model-facing tools exposed by every Environment target:

- `bash`
- `read_file`
- `write_file`
- `edit_file`
- `glob`
- `grep`

The session owns exactly one logical Environment, and provider selection comes
from the pinned profile. Lifecycle is automatic and available through the
control-plane Environment API rather than model-facing create/list/status tools.

## Architecture

### Platform

`crates/contracts/src/session_sandbox.rs` owns the neutral configuration,
provider interface, response values and provider registration.
`crates/capabilities/src/session_sandbox.rs` owns lifecycle orchestration and
state persistence through the runtime store interface. Hosted deployments keep
logical and physical environment rows in server-owned PostgreSQL storage.

`crates/capabilities/src/capabilities/session_sandbox.rs` exposes the capability;
`environment_tools.rs` implements the stable tool vocabulary. Tool execution
resolves the configured provider and delegates through the trait. After a
completed shell or file mutation, the provider checkpoint is persisted before
the tool result is returned to the runtime.

Initial session files are copied into a newly created managed workspace exactly
once before the first tool call. The managed target removes the inherited VFS
file and Bashkit shell capabilities, so every tool sees the same `/workspace`.

### Integrations

Provider implementations live in integration crates and register with:

`everruns_contracts::SessionSandboxProviderPlugin`

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

The runtime emits `environment.instance_lost` before replacement and
`environment.recovered` after the new generation is persisted. Their payloads
contain only non-secret logical/provider/incarnation identifiers and explicitly
report `process_state_lost: true`.

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

Recovery reconciles `sandboxes.current_checkpoint_id` against
`durable_tool_results` before resume. A checkpoint produced by a tool call that
never settled is detached and the provider recovery pointer is rewound to the
previous committed revision. This closes the crash window without reopening the
frozen core transaction boundary.

Hosted PostgreSQL deployments persist the current physical incarnation in
`sandbox_instances` and point to it from the durable logical `sandboxes` row.
Replacing a missing provider resource increments the logical generation and
retires the former incarnation. Every subsequent state write and delete carries
that generation; a late response from the retired provider resource is rejected
instead of becoming current again. Existing encrypted `session_sandbox` records
are adopted lazily and removed after the first successful database write.

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

Hosted managed-environment state is queryable product state in `sandboxes`,
`sandbox_instances`, and `sandbox_checkpoints`. These internal table names
predate the public Environment vocabulary introduced by the execution
environment surface; they do not create a second product resource. The Daytona
sandbox id belongs to a disposable incarnation and may change after recovery.

Framework, remote, and in-memory hosts that do not install the hosted state
store continue to use the encrypted `session_sandbox` secret as a compatibility
fallback. That fallback is not the hosted control-plane authority.

Provider-owned remote resources should also register leased resources so the
session resource registry and cleanup infrastructure can see them. Daytona does
this through the existing leased-resource store.

## Non-goals

- multiple managed sandboxes per session
- public/documentation-site docs before the feature stabilizes
- durable distributed idle scheduling in the first experimental version
