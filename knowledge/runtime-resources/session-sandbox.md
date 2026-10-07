---
type: Specification
title: "Managed Sandbox Runtime"
description: "Provider-neutral managed Sandbox capability, workspace durability, and recovery lifecycle."
tags:
  - everruns
  - runtime-resources
---
# Managed Sandbox Runtime

`session_sandbox` is the internal runtime capability for a managed, session-owned
Sandbox. Sandbox Templates are the reusable product configuration surface.

## Goal

Provide one sandbox per session with a provider-neutral tool surface and
server-managed lifecycle:

- auto-start on session creation
- pause after session idle timeout
- resume on next sandbox tool use
- replacement and workspace restore when the physical provider sandbox is lost
- optional one-time init commands
- provider pluggability (Daytona first)

Sandbox Templates are available without a separate feature flag.
PostgreSQL deployments must configure `SECRETS_ENCRYPTION_KEY` so lifecycle
state and provider connections can be resolved; the canonical local startup
supplies its stable development key. Daytona execution also requires an
organization or user Daytona connection.

## Capability

Capability id: `session_sandbox`

New sessions receive this configuration from their resolved Sandbox Template.
Direct capability configuration remains supported for existing data:

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

Stable model-facing tools exposed by every primary Sandbox target:

- `bash`
- `read_file`
- `write_file`
- `edit_file`
- `glob`
- `grep`

The Session owns at most one logical primary Sandbox, and provider selection
comes from its pinned Sandbox Template spec. Lifecycle is automatic and
available through the control-plane Sandbox API rather than model-facing
create/list/status tools.

## Architecture

### Platform

`crates/contracts/src/session_sandbox.rs` owns the neutral configuration,
provider interface, response values and provider registration.
`crates/capabilities/src/session_sandbox.rs` owns lifecycle orchestration and
state persistence through the runtime store interface. Hosted deployments keep
logical Sandbox and physical instance rows in server-owned PostgreSQL storage.

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

`crates/integrations/src/daytona/session_sandbox_provider.rs`

Modal (`crates/integrations/src/modal/session_sandbox.rs`, provider `modal`) is
the second. Modal has no stop/start, so pause snapshots the filesystem into a
Modal image and terminates the sandbox, and resume boots a new sandbox from that
image: the external id changes on every resume. A sandbox Modal ended without a
pause reports `Lost` unless an earlier pause left a snapshot. It keeps no
Everruns recovery volume, so Sandbox Templates allow only `provider_snapshot`
durability for it, and it is offered only at development grade while the
integration is experimental. See [Modal Sandboxes](../integrations/modal.md).

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

The runtime emits `sandbox.instance_lost` before replacement and
`sandbox.recovered` after the new generation is persisted. Their payloads
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

Current implementation uses in-process timers. Durable multi-instance
scheduling can replace it without changing the Sandbox contract.

## State and cleanup

Hosted managed-Sandbox state is queryable product state in `sandboxes`,
`sandbox_instances`, and `sandbox_checkpoints`. The Daytona sandbox id belongs
to a disposable incarnation and may change after recovery.

### Fleet view and history

The Sandboxes page (`/sandboxes`, `GET /v1/sandboxes[/stats|/timeline|/{id}]`,
[`domains/sandboxes`](../../crates/server/src/domains/sandboxes/commands.rs))
lists every logical Sandbox in an organization across providers. It reads
three things the hosted store keeps:

- `sandboxes` rows outlive their Session. Deleting a Session marks its
  Sandboxes deleted and keeps the Session title and Agent, so deleted compute
  stays explainable. Deleted rows are purged after
  `SANDBOX_HISTORY_RETENTION_DAYS` (default 30,
  [`sandbox_history_retention.rs`](../../crates/server/src/sandbox_history_retention.rs)).
- `sandbox_state_transitions` is an append-only log written by a database
  trigger on every `observed_state` or `generation` change, so running time,
  pauses and rebuilds can be drawn without each writer remembering to log.
- "Needs attention" reasons (lost, failed, init commands failed, running with
  no activity for an hour, provider cleanup failed) are computed in the fleet
  query, so they filter and page like any other column.

In-process targets (virtual filesystem, host) have no provider resource and
are hidden unless `include_in_process` is set. The fleet is read-only;
pause, resume and delete go through the Session's Sandbox endpoint.

Framework, remote, and in-memory hosts that do not install the hosted state
store continue to use the encrypted `session_sandbox` secret as a compatibility
fallback. That fallback is not the hosted control-plane authority.

Provider-owned remote resources should also register leased resources so the
session resource registry and cleanup infrastructure can see them. Daytona does
this through the existing leased-resource store.

## Non-goals

- more than one implicit primary Sandbox per Session; explicitly addressed
  resource Sandbox fleets are a separate capability
- durable distributed idle scheduling in the first experimental version
