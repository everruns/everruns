---
type: Specification
title: "Sandbox abstraction"
description: "Provider-neutral sandbox model combining filesystem, compute, lifecycle, and checkpoint semantics."
tags:
  - everruns
  - harnesses
  - sandbox
  - runtime-resources
---

# Sandbox abstraction: filesystem plus compute

Status: active proposed architecture. Replaces the experimental `session_sandbox` direction and
consolidates the provider-specific sandbox capabilities.

## Decision

Give every agent session one **logical sandbox**: a provider-neutral working
filesystem plus an optional compute surface. The agent version selects a
sandbox profile; the control plane creates and reconciles disposable physical
incarnations of that logical sandbox.

```text
AgentVersion ── profile ──> Session ──> Sandbox (logical, durable)
                                          │             │
                                  current checkpoint    │ reconcile
                                          │             ▼
                              Everruns durable store   Incarnation g17
                                                       (disposable)
                                                           │
                                         ┌─────────────────┴─────────────────┐
                                         │                                   │
                                  working filesystem                      compute
                             read/write/edit/glob/grep             exec/stream/cancel
                                         │                                   │
                                         └────────── same namespace ──────────┘
                                                           │
                                          Bashkit | Daytona | future provider
```

A physical VM, container, process, or provider resource may disappear at any
time. The Agent and logical Sandbox do not die with it. Everruns provisions a
new incarnation, restores the last committed working-state checkpoint, reapplies
the pinned bootstrap revision, and resumes the durable agent loop. Continuing
work must not depend on the survival of a provider resource ID.

The model sees the stable tools `bash`, `read_file`, `write_file`, `edit_file`,
`glob`, and `grep`. It does not create sandboxes, choose providers, pass provider
resource IDs, or manage idle lifecycle. Provider and lifecycle controls belong
to the API/UI and the control plane.

Default ownership is **one sandbox per session**. Selecting a provider on an
Agent does not mean all sessions of that Agent share one mutable machine.
Workspace-scoped sandboxes may be added as an explicit advanced mode, with an
exclusive writer lease.

## Why now

Everruns already has most of the pieces, but they overlap:

- `SessionFileSystem` + `MountFs` provide a durable database/host-backed VFS.
- `bashkit_shell` executes against that VFS directly.
- `session_sandbox` provides one Daytona-backed sandbox with generic
  `sandbox_*` tools and stores its record in a session secret.
- Daytona, E2B, Deno, Sprites, `container_sandbox`, and `docker_container` each
  expose their own create/exec/file/list/manage tools and state format.
- `coding-session-sandbox` deliberately exposes both session files and sandbox
  files, then relies on prompt text to stop the model from confusing them.

The current experimental abstraction therefore standardizes tool names, but
not the filesystem. `sandbox_exec` and `sandbox_read_file` address Daytona while
`read_file`, AGENTS.md, skills, output persistence, and the Workspace API address
the Everruns VFS. That is split-brain state, not one sandbox.

Lifecycle is similarly fragmented. Saved state has only `running` and `paused`,
while providers distinguish provisioning, started, stopped, archived,
checkpointed, failed, and deleted. In-process idle timers are lost on restart,
and non-secret sandbox identity is hidden in encrypted session secrets rather
than represented as queryable control-plane state.

## SOTA findings

Research current as of 2026-08-09.

### Vercel eve

eve treats the sandbox as the agent's single filesystem and shell. Built-in file
and bash tools run in the app runtime but proxy into the same sandbox. The
sandbox is an adapter: local development may use Docker, microsandbox, or
just-bash, while deployment uses Vercel Sandbox. Agent execution remains outside
the VM, so workflow durability and sandbox lifecycle can evolve independently.

Vercel also separates a long-lived sandbox identity from a running VM session.
Stopping a persistent sandbox checkpoints its filesystem; resuming boots a new
VM from that checkpoint. Storage that must outlive the sandbox is a separate
mounted Drive.

Sources:

- <https://vercel.com/blog/introducing-eve>
- <https://github.com/vercel/eve/blob/main/docs/concepts/default-harness.md>
- <https://github.com/vercel-labs/open-agents>
- <https://vercel.com/kb/guide/vercel-sandbox-duration-and-persistence>

### LangChain Managed Deep Agents

Deep Agents defines a backend as the filesystem contract. A sandbox backend
extends it with `execute`, so file tools and shell execution share one backend.
Lifecycle scope is explicit: thread-scoped is the common isolated default;
assistant-scoped is an opt-in shared environment. Persistent memory is routed
to separate paths with a composite backend instead of being confused with
scratch state.

Sources:

- <https://docs.langchain.com/oss/python/deepagents/overview>
- <https://docs.langchain.com/oss/python/deepagents/backends>
- <https://docs.langchain.com/oss/python/deepagents/going-to-production>
- <https://www.langchain.com/blog/introducing-managed-deep-agents>

### Claude Managed Agents

Claude separates Agent, Environment, and Session. The Agent is versioned
behavior, the Environment is reusable sandbox configuration, and each Session
gets an isolated sandbox instance. The same built-in toolset handles bash and
file operations inside that sandbox. When a session idles, its sandbox is
checkpointed and later restored.

Claude also makes the retention boundary explicit: conversation history can
outlive sandbox state, while sandbox checkpoints currently expire after 30
days. Uploaded inputs are independent file resources mounted read-only, and
important results are written to outputs rather than relying forever on the
sandbox disk.

Sources:

- <https://platform.claude.com/docs/en/managed-agents/overview>
- <https://platform.claude.com/docs/en/managed-agents/quickstart>
- <https://platform.claude.com/docs/en/managed-agents/environments>
- <https://platform.claude.com/docs/en/managed-agents/events-and-streaming>
- <https://platform.claude.com/docs/en/managed-agents/files>

### Bashkit and Daytona

Bashkit already fits the required provider contract: virtual filesystem,
in-process bash compute, resource limits, default-deny networking, and snapshots.
Its important limitation is intentional: it is not a real Linux process
environment and cannot install arbitrary packages or run arbitrary native
binaries.

Daytona supplies the other end of the spectrum. Its filesystem survives
stop/start; pause/resume may additionally preserve memory for VM classes; and
volumes persist independently of a sandbox. These are different guarantees and
must be capabilities, not forced through one universal `pause` operation.

Sources:

- <https://github.com/everruns/bashkit>
- <https://docs.rs/bashkit/latest/bashkit/>
- <https://www.daytona.io/docs/en/persistence/>
- <https://www.daytona.io/docs/en/volumes/>

## Product model

### Agent version: desired sandbox profile

An Agent version carries immutable desired configuration, resolved through the
normal Harness -> Agent -> Session layering and pinned when the session starts.

```json
{
  "provider": "bashkit",
  "scope": "session",
  "provision": "lazy",
  "runtime": {
    "image": null,
    "cpu": null,
    "memory_mb": 512,
    "disk_mb": 1024
  },
  "network": {
    "mode": "deny",
    "allowed_hosts": []
  },
  "lifecycle": {
    "idle_after_seconds": 180,
    "idle_action": "checkpoint_and_stop",
    "retention_seconds": 2592000
  },
  "bootstrap": {
    "revision": "sha256:...",
    "commands": []
  },
  "provider_options": {}
}
```

`provider_options` is an escape hatch, not the main contract. Common security,
resource, bootstrap, and lifecycle fields remain portable and validated by the
platform. The resolved profile snapshot is stored on the Sandbox row so an
Agent update cannot silently change a running session's environment.

### Sandbox: durable logical resource

Sandbox identity and observed lifecycle become first-class database state, not
a session secret.

```text
sandboxes
  id                         sandbox_<uuid>
  org_id
  session_id?                exactly one owner today
  workspace_id?              reserved for explicit shared scope
  provider
  profile_snapshot           non-secret JSON
  profile_revision
  desired_state              ready | stopped | deleted
  observed_state             see state machine below
  generation                 current incarnation/fencing counter
  current_checkpoint_id?
  owner_user_id?             credential/connection owner
  last_activity_at
  idle_deadline_at?
  operation_started_at?
  last_error_code?
  last_error_message?
  created_at / updated_at / deleted_at?
```

Physical provider identity lives in a separate incarnation record:

```text
sandbox_instances
  id
  sandbox_id
  generation
  provider_resource_id       non-secret external identity
  provider_state             non-secret, bounded JSON
  observed_state
  provisioned_at
  last_seen_at?
  lost_at?
  stopped_at?
  deleted_at?
```

Every replacement increments `generation`. Calls and leases carry that fencing
value, so a late response from a lost incarnation cannot overwrite current
state. Historical incarnation rows retain enough identity for cleanup and
incident investigation.

Provider credentials are resolved at operation time from the recorded
connection owner. Tokens, injected environment values, preview tokens, and
signed URLs never enter `profile_snapshot`, `provider_state`, a checkpoint
manifest, or leased-resource metadata.

Leased resources remain useful, but only as cleanup machinery for the external
provider resource. The Sandbox row is authoritative product state; its lease is
a subordinate cleanup record.

### Sandbox checkpoint

Conversation checkpoints and sandbox checkpoints are unrelated records.

```text
sandbox_checkpoints
  id
  sandbox_id
  generation
  source_tool_call_id?
  kind                  provider_native | portable_workspace
  provider_ref?         snapshot/checkpoint id
  object_ref?           portable tar manifest in object storage
  workspace_revision
  size_bytes?
  expires_at?
  created_at
```

- A provider-native checkpoint is only a fast resume path. It is insufficient
  when the provider resource or its snapshots disappear together.
- A portable workspace checkpoint is an Everruns-owned, bounded archive or
  content-addressed manifest of the working filesystem. It is the recovery path
  for physical sandbox loss, provider migration, disaster recovery, and
  retention beyond a provider snapshot.
- Neither format should be the only home of a user-requested deliverable.
  Published outputs live in Everruns durable file/object storage.

### Agent-step commit boundary

The durable agent loop and working filesystem advance together:

```text
checkpoint N + committed conversation state
                │
                ├─ ensure incarnation ready from checkpoint N
                ├─ execute tool call
                ├─ capture durable working checkpoint N+1
                └─ atomically commit tool result + checkpoint N+1 reference
```

A completed mutating tool result is not committed to the conversation until its
working state is recoverable outside the physical incarnation. Upload the
checkpoint first, then commit the tool result and checkpoint reference in one
database transaction; unattached uploads are safe garbage-collection orphans.

Everruns reaches that invariant by reconciliation rather than a shared
transaction (EVE-870). A shared transaction would need a co-commit seam through
`everruns-core`, whose 0.18 public boundary is frozen (EVE-906), and the
saga-shaped alternative satisfies the same invariants without reopening it. So
the checkpoint is attached first and the tool result settles after, and the
window between them is closed on the way back in: resume compares the
authoritative checkpoint's `source_tool_call_id` against `durable_tool_results`
and, unless that call settled, rolls the pointer back to the previous attached
checkpoint. The rejected revision is detached rather than deleted, which returns
it to the collectable pool. Reconciliation only ever rejects the current
checkpoint, only on a positive "not settled" answer from durable storage, and
never on missing information — over-eager rollback discards real work, which is
worse than the drift it would prevent. See
`reconcile_session_sandbox_checkpoint` in `crates/platform/src/session_sandbox.rs`.

All `bash` executions are treated as mutating because a shell command can alter
arbitrary files, including when it exits non-zero. Generic write/edit/delete
operations use the same boundary. Read-only file operations do not require a
new checkpoint. Background processes require periodic checkpoints with an
explicit recovery-point objective plus a final checkpoint when they finish.

If an incarnation disappears during a command, that command has no committed
tool result. Everruns restores checkpoint N in a new incarnation and resumes the
agent with an interruption event. The model can inspect and retry. External
network side effects cannot be rolled back by filesystem recovery; their
idempotency remains the owning tool's responsibility.

### Three kinds of state

| State | Owner | Persistence contract |
|---|---|---|
| Conversation/events | Session runtime | Durable until session retention/deletion |
| Active worktree and processes | Physical incarnation | Disposable; may disappear without warning |
| Recoverable working filesystem | Logical Sandbox | Everruns-owned checkpoint through the last committed mutating tool call |
| Published inputs, outputs, memory, and artifacts | Everruns Workspace/object store | Independent of sandbox lifecycle |

This distinction is required even when a provider happens to make them look the
same. For Bashkit, the working filesystem may directly use the Everruns VFS, so
checkpoint/export is effectively free. For Daytona, the active filesystem is
remote but its committed worktree is mirrored to an Everruns-owned checkpoint.
Published outputs remain a separate, intentional durability contract.

## Runtime contracts

Do not make every provider implement a wide trait containing every lifecycle
operation. Split the required data plane from capability-negotiated extensions.

```rust
#[async_trait]
pub trait SandboxProvider: Send + Sync {
    fn id(&self) -> &'static str;
    fn capabilities(&self) -> SandboxCapabilities;

    async fn provision(&self, request: ProvisionRequest) -> Result<ProviderSandbox>;
    async fn inspect(&self, sandbox: &ProviderSandbox) -> Result<ProviderStatus>;
    async fn start(&self, sandbox: &ProviderSandbox) -> Result<ProviderSandbox>;
    async fn stop(&self, sandbox: &ProviderSandbox, mode: StopMode) -> Result<ProviderSandbox>;
    async fn delete(&self, sandbox: &ProviderSandbox) -> Result<()>;

    async fn connect(&self, sandbox: &ProviderSandbox) -> Result<SandboxHandle>;
}

#[async_trait]
pub trait SandboxCheckpointProvider: Send + Sync {
    async fn checkpoint(
        &self,
        sandbox: &ProviderSandbox,
        request: CheckpointRequest,
    ) -> Result<ProviderCheckpoint>;

    async fn restore(
        &self,
        checkpoint: &ProviderCheckpoint,
        request: RestoreRequest,
    ) -> Result<ProviderSandbox>;
}

pub struct SandboxHandle {
    pub filesystem: Arc<dyn WorkingFileSystem>,
    pub compute: Option<Arc<dyn SandboxCompute>>,
}

#[async_trait]
pub trait SandboxCompute: Send + Sync {
    async fn exec(&self, request: ExecRequest, sink: OutputSink) -> Result<ExecResult>;
    async fn cancel(&self, execution_id: &str) -> Result<()>;
}
```

`WorkingFileSystem` should be the scoped successor to `SessionFileSystem`:
byte-first, rooted, and not passed a `SessionId` on every operation. It covers
stat/list/read ranges/write with compare-and-set/mkdir/remove/rename/glob/grep.
The existing `SessionFileSystem` gets a transitional scoped adapter; `MountFs`
continues to own path normalization and presentation.

Required provider capabilities:

- working filesystem;
- shell/script execution when the selected profile enables `bash`;
- inspect, start/ensure-ready, stop, and delete;
- streaming output, timeout, and cancellation or an honest unsupported result.

Negotiated optional capabilities:

- provider-native filesystem checkpoint;
- memory checkpoint/pause;
- portable archive import/export acceleration;
- persistent background processes;
- PTY;
- preview ports;
- network allowlist enforced by the provider;
- persistent/shared volume mounts;
- fork/clone;
- resource resize.

Tools and UI must be assembled from this capability set. An unsupported
operation is absent, not emulated with misleading semantics.

## Tool and filesystem routing

`ToolContext` currently has one `file_store`, which implicitly means both
working files and durable session files. Split it:

```text
working_files   -> active SandboxHandle.filesystem
artifact_store  -> durable Everruns Workspace/object storage
compute         -> active SandboxHandle.compute
```

Rules:

1. `bash` and the generic file tools always target `working_files`.
2. AGENTS.md, skills, initial project files, and uploaded inputs are seeded or
   mounted into the working filesystem during bootstrap.
3. Full tool output, published deliverables, and explicit artifacts go to
   `artifact_store` and carry recovery pointers.
4. A remote provider must never leave generic file tools on the database VFS
   while bash runs against its remote disk.
5. Provider selection changes no model-facing tool names or prompt.
6. Lifecycle tools are not part of the default model toolset. Status/reset are
   control-plane API/UI actions. An expert capability may expose them when a
   workflow genuinely manages multiple sandboxes as data.

For Bashkit, `working_files` and `artifact_store` may point to the same mounted
Everruns filesystem. For Daytona, they are distinct: the worktree is mirrored
to recoverable Sandbox checkpoints at the agent-step commit boundary, while
publication to the artifact store remains explicit. This is an implementation
difference, not a different model-facing filesystem contract.

## Lifecycle and reconciliation

### Desired vs observed state

Do not trust a locally saved `running` flag. Persist intent separately and
reconcile it with provider observations.

```text
observed_state:
  absent
  provisioning
  starting
  ready
  checkpointing
  stopping
  stopped
  restoring
  lost
  recovering
  failed
  deleting
  deleted
  unknown
```

`desired_state` stays small: `ready`, `stopped`, or `deleted`. Provider-specific
states map into the observed state plus non-secret metadata.

### SandboxManager

All tool, API, idle, and cleanup paths call one durable manager:

```text
ensure_ready(sandbox_id, cause) -> fenced SandboxHandle
mark_activity(sandbox_id)
request_stop(sandbox_id, cause)
request_reset(sandbox_id)
request_delete(sandbox_id)
reconcile(sandbox_id)
```

The manager:

- serializes provision/start/stop/checkpoint/delete with a DB claim;
- uses `generation` as a fencing token;
- gives provider mutations stable idempotency keys;
- refreshes external resource leases while work is active;
- records operation errors without losing the last provider identity;
- schedules idle transitions durably, replacing in-process `tokio::sleep`;
- re-inspects `unknown`, `failed`, or stale in-progress records;
- treats provider `not found`, an expired instance, or a permanently unhealthy
  instance as physical loss rather than logical Sandbox deletion;
- publishes lifecycle events for API/UI and observability.

An exec first calls `ensure_ready`, then obtains a short activity lease. Idle
checkpoint/stop cannot begin while an activity lease is held. Cancellation ends
the execution and releases the lease; it does not imply sandbox deletion.

### Physical-loss recovery

When the current incarnation is missing or unrecoverable, the manager:

1. Marks the incarnation `lost` and fences its generation.
2. Loads the latest committed portable workspace checkpoint.
3. Provisions a new provider instance from the pinned profile.
4. Imports or mounts the recoverable worktree.
5. Applies the pinned bootstrap revision idempotently.
6. Verifies the restored workspace revision and required sentinel files.
7. Publishes the new incarnation as `ready` and resumes the durable agent loop.

The recovery event tells the Agent that physical compute was replaced and that
any uncommitted command, background process, server, PTY, in-memory interpreter,
or shell state was lost. "Continue working" guarantees conversation and files,
not RAM. Durable session tasks may restart their process from task metadata;
ordinary background processes do not silently pretend to have survived.

A provider is eligible as a default durable-agent backend only if Everruns can
maintain this recovery contract through at least one of:

- an Everruns-owned durable volume mounted as the worktree;
- incremental filesystem change export after each mutating step; or
- a portable full-worktree checkpoint after each mutating step.

Provider-native snapshots may optimize recovery but do not replace this
independent durability path.

### Bootstrap

Replace `init.commands` + `init_completed_at` with a versioned, idempotent
bootstrap revision. Record each revision's outcome and log artifact. A new
revision runs once against an existing sandbox. Restore and migration can verify
the revision rather than guessing whether a command partially succeeded.

### Shared scope

Session scope is the only initial production mode. A future workspace-scoped
sandbox must have:

- an exclusive active writer lease by default;
- fencing on every mutating call;
- an explicit concurrent-read policy;
- reset/fork semantics that cannot surprise other attached sessions;
- UI that shows which session currently owns execution.

Do not infer shared mutable compute merely because multiple sessions attach to
one Everruns Workspace.

## Provider mappings

### Bashkit

- Provision: create/adopt the sandbox row; no external resource.
- Filesystem: current mounted Everruns `SessionFileSystem` through a scoped
  adapter.
- Compute: construct Bashkit per exec against that filesystem.
- Stop/start: logical no-op; no billed warm compute exists.
- Checkpoint: VFS is already durable. Bash interpreter snapshots are optional
  if persistent shell variables ever become a product requirement.
- Capabilities: no native processes, packages, PTY, ports, or arbitrary Linux
  binaries. Network remains default-deny through Everruns egress.

Rebuilding Bashkit for every command is acceptable because shell variables and
background processes are not currently part of the contract. Files are the
durable state.

### Daytona

- Provision: one provider sandbox from the pinned profile.
- Filesystem: Daytona API rooted at the configured provider worktree (currently
  `/home/daytona/workspace`; logical path translation is internal).
- Compute: streaming Daytona exec.
- Stop: preserve filesystem; pause/memory preservation only when the selected
  Daytona class advertises it.
- Checkpoint: provider-native snapshot for fast restore. An Everruns-owned
  portable workspace checkpoint is required for the durable-agent profile
  unless the worktree uses an Everruns-owned durable volume.
- Volumes: optional persistent mount capability, not assumed by the core.
- Lease: cleanup protection for the external sandbox; not product state.

#### Daytona recovery volume

Do not use a Daytona Volume as the live worktree by default. Daytona
Volumes are FUSE mounts backed by object storage: they are durable across
sandbox deletion, but are slower than the sandbox-local filesystem and use
last-writer-wins rather than transactional writes. The default layout is:

```text
/home/daytona/workspace    fast, disposable Daytona-local worktree
/mnt/everruns-recovery     durable volume mount used for checkpoints
```

The persistent binding belongs to the logical Everruns Sandbox:

```text
provider:      daytona
storage_kind: daytona_volume | external_object_store
volume_id:     <shared volume or bucket binding>
subpath:       org/<org_id>/sandboxes/<sandbox_id>
mount_path:    /mnt/everruns-recovery
head_revision: <committed workspace revision>
```

Use one shared volume per organization, region, or storage policy and a unique
subpath per logical Sandbox. Do not allocate one Daytona Volume per Sandbox:
Daytona limits the number of volumes, and its subpath isolation is designed for
this sharing pattern. A replacement physical instance receives the same
`volume_id` and `subpath` in its create request:

```json
{
  "volumes": [
    {
      "volumeId": "vol_everruns_recovery",
      "mountPath": "/mnt/everruns-recovery",
      "subpath": "org/org_123/sandboxes/sbx_456"
    }
  ]
}
```

Provision and recovery are therefore:

1. Resolve or create the shared recovery volume and allocate the stable
   Sandbox subpath.
2. Create a Daytona sandbox with that volume binding.
3. Restore `head_revision` from the mount into the new local worktree.
4. Run the versioned, idempotent bootstrap.
5. Publish the new SandboxInstance generation as ready.

After every completed mutating tool call, a helper inside the sandbox writes a
new immutable workspace revision to the mount. It writes content and manifest
first, a `COMPLETE` marker next, and a non-authoritative convenience `HEAD`
last. Only after Everruns has verified that revision may it commit the tool
result and revision pointer in the database. If the physical sandbox dies
partway through a command or checkpoint, the unpublished revision is ignored
and the replacement restores the preceding persisted revision pointer.

The first implementation may store a full compressed workspace archive per
revision, excluding rebuildable caches such as `node_modules`, `target`, and
`.venv`. The scalable implementation is content-addressed: immutable blobs by
hash plus an immutable manifest per revision, so only changed bytes are
uploaded. Both schemes must preserve file type, mode, symlink target, and
deletions; directory rename atomicity must not be assumed on the FUSE mount.

Exactly one fenced mutating activity may own a logical Sandbox at a time. That
is required independently of the provider and is especially important because
concurrent Daytona Volume writes are last-writer-wins.

A Daytona-managed Volume is independent of a Daytona sandbox instance, which
is sufficient for ordinary instance loss. It is not independent of Daytona as
a provider. For the durable-agent profile, prefer Daytona's external-storage
mount backed by an Everruns-controlled S3-compatible bucket, or replicate each
committed revision there. Provider snapshots and Daytona-managed Volumes remain
useful restore accelerators, never the only authoritative copy when the profile
promises provider-independent recovery.

The first implemented slice uses the current session-sandbox state record as
the logical binding: it gets or creates one shared `everruns-recovery` Daytona
Volume, mounts `sessions/<session_id>`, stores the authoritative revision in
provider state, and replaces a physical Daytona sandbox after a `404`. This
delivers ordinary instance-loss recovery while the first-class Sandbox tables
and a transaction spanning checkpoint selection plus durable tool-result commit
remain follow-up architecture work.

### Later providers

E2B, Deno, Sprites, container/Docker, Vercel Sandbox, Cloudflare Sandbox, and
other providers implement the same driver contracts. Their raw APIs remain
inside integration crates. Provider differences surface through capabilities
and profile validation, not different tools given to the model.

## Deprecations

### Deprecate now

- Experimental `session_sandbox` capability and its `sandbox_*` model tools.
  Its code is useful as a spike, but its filesystem and state ownership are the
  wrong final boundaries.
- `coding-session-sandbox` and `coding-daytona`. Replace them with one coding
  harness whose sandbox profile is configurable.
- `docker_container`. It duplicates `container_sandbox`, uses the Docker CLI,
  and advertises host networking. Remove it before graduating a Docker driver.
- Storing non-secret sandbox identity/lifecycle records in session secrets.
- In-process session-idle timers as the authoritative lifecycle mechanism.

### Clarify experimental status

The [Lua execution work](../execution/lua-execution.md) is an experiment,
not an adopted product plan. Its current "supersede `bashkit_shell`" and
"Migration" wording overstates that status and should be reframed as a
hypothesis plus evaluation results. Lua may become an optional interpreter or
runtime feature, but this sandbox proposal makes no decision to replace
Bashkit. Bashkit and Lua are also not competing platform abstractions: Bashkit
can implement the sandbox contract, while Lua is a language/runtime choice.

### Compatibility window

- Treat `bashkit_shell` config as a compatibility alias for
  `sandbox.provider = "bashkit"`; stop presenting it as a separate execution
  architecture.
- Keep raw Daytona/E2B/Deno/Sprites capabilities temporarily for workflows that
  intentionally create multiple sandboxes. Mark them advanced/deprecated for
  normal agent execution and remove them from built-in harnesses.
- Keep `/v1/sessions/{session_id}/sandbox` as a compatibility route backed by
  the new Sandbox resource. Add first-class `/v1/sandboxes/{sandbox_id}` APIs.
- Lazily adopt existing `session_sandbox` secret records into Sandbox rows on
  first read. Preserve the provider resource ID; do not recreate a live remote
  sandbox during migration.
- Map `container_sandbox` to the Docker provider after the driver exists, then
  deprecate its create/list/manage tool surface.

## Migration plan

### Phase 1: state and contracts

1. Add Sandbox, SandboxCheckpoint, and durable idle-operation persistence.
2. Add SandboxInstance generations, `SandboxManager`, provider capabilities,
   `WorkingFileSystem`, and `SandboxCompute`.
3. Implement the Bashkit provider first. Route existing `bash` and generic file
   tools through the active handle with no behavior regression.
4. Split `ToolContext` working files from artifact storage and bind committed
   tool results to a working-filesystem revision.

Exit gate: Generic/Bashkit tests prove that bash and every generic file tool see
the same bytes and paths; restart tests prove lifecycle intent survives.

### Phase 2: Daytona

1. Port the existing Daytona client into the driver contract.
2. Move state from session secrets into Sandbox rows and retain lease cleanup.
3. Add checkpoint/restore, per-mutating-step portable recovery, bootstrap
   revision, physical-loss replacement, and durable idle reconciliation.
4. Point the existing session sandbox API at `SandboxManager`.

Exit gate: a session can write via `write_file`, read via bash, idle/stop,
restart Everruns, resume, and read the same file. The inverse write/read path
must also pass. A second test forcibly deletes the Daytona sandbox between
turns; Everruns must create a new provider resource, restore the committed
worktree, and continue the same session.

### Phase 3: product migration

1. Replace sandbox-specific coding harnesses with one configurable coding
   harness.
2. Default new sessions to session scope and lazy provisioning.
3. Add Sandbox status/reset/delete UI and published-artifact UX.
4. Mark old capabilities and tool names deprecated with migration warnings.

### Phase 4: provider consolidation

Port E2B, Deno, Sprites, and container sandbox behind the driver contract.
Remove provider-specific tools from standard installations after telemetry
shows no active built-in use and the documented deprecation window ends.

## Acceptance criteria

- One model-visible working filesystem: all generic file tools and `bash` are
  coherent for every provider.
- Provider choice is pinned in session/Sandbox state and invisible to the
  model-facing tool contract.
- Server/worker restart cannot lose an idle stop, checkpoint, delete, or
  in-progress reconciliation request.
- Repeated provision/start/stop/delete calls are idempotent under retries.
- A stale worker cannot mutate a newer Sandbox generation.
- Forcibly deleting the active physical provider instance does not terminate
  the Agent or logical Sandbox.
- Everruns can provision a replacement incarnation and restore every file from
  the last committed mutating tool result.
- A tool result is never committed while its corresponding worktree revision
  exists only on the physical provider instance.
- Sandbox identity is queryable without decrypting session secrets.
- No bearer credential appears in Sandbox rows, checkpoints, events, logs, or
  resource metadata.
- Provider capabilities accurately control available API/UI operations.
- Important outputs survive sandbox deletion through explicit durable
  publication.
- Session deletion requests sandbox deletion but cleanup continues even if the
  session row disappears.
- Evals cover Bashkit and at least one real remote provider with the same task
  and grader.

## Explicit non-goals for the first version

- Multiple model-managed sandboxes per session.
- Shared mutable sandboxes across sessions.
- A universal long-lived process or PTY contract.
- Transparent live mounting of the Everruns PostgreSQL VFS into every remote
  provider.
- Cross-provider memory/process checkpoint portability.
- Zero-loss recovery for a command interrupted midway; recovery resumes from
  the last committed tool/checkpoint boundary.
- Hiding real provider differences behind fake pause, fork, port, or volume
  behavior.
