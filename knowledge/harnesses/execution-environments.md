---
type: Proposal
title: "Execution environments"
description: "Two-axis model separating where an agent's commands run from what those commands may touch, so no-sandbox, Bashkit, Daytona, and a real machine are one contract."
tags:
  - everruns
  - yolop
  - harnesses
  - sandbox
  - execution
---

# Execution environments

Status: proposal. Extends, does not replace,
[Sandbox Abstraction](sandbox-abstraction.md). That concept solved the durable
logical sandbox: one working filesystem, provider-neutral drivers, checkpoints,
and physical-loss recovery. This proposal adds the two things it left out, then
folds Yolop into the same contract:

1. an environment where nothing is contained (the machine the agent is already
   running on), and
2. a containment axis, so "no sandbox", "kernel-contained host", and "remote
   VM" are positions on one scale rather than unrelated products.

## Problem

### The word "sandbox" carries two meanings

Everruns uses it for **where code runs**, and answers that question by which
capability the harness enables. Bashkit, `container_sandbox`, Daytona, and E2B
are four independent capabilities with four tool families and four state
formats. `SessionSandboxProvider` (`crates/platform/src/session_sandbox.rs`) is
the newer provider-neutral attempt at the same question, but it is behind an
internal feature flag and Daytona is its only implementation. Containment is not
a field anywhere: it is whatever the chosen capability happens to give, so
Bashkit is default-deny by construction while a Daytona VM is wide open inside
itself.

Yolop uses it for **what code may touch**. `SandboxProvider`
(`src/exec/sandbox.rs`, `SandboxMode` in `src/config/mod.rs`) always runs on the
local machine and varies only the kernel policy: `read-only`,
`workspace-write`, `danger-full-access`, enforced with Seatbelt and Landlock.

Neither system can express the other's question. Everruns cannot say "run on
this box but deny the network". Yolop cannot say "run this somewhere else".
Both are asked for exactly that.

### Selecting an environment means selecting a harness

`coding-container`, `coding-daytona`, and `coding-session-sandbox` differ in one
capability each, and then repeat roughly a hundred lines of near-identical
system prompt with provider tool names spelled into the text
(`sandbox_exec` vs `daytona_exec` vs `sandbox_read_file`). Changing where a
session runs currently means changing its behavior, its prompt, and its tool
names at once.

### Five tool namespaces for the same six operations

`read_file`/`write_file`/`edit_file` (session VFS), `bash` (Bashkit),
`sandbox_*` (container and session sandbox), `daytona_*`, `e2b_*`. The model
learns a different vocabulary per provider for read, write, exec, and lifecycle.

### There is no honest "no sandbox"

The nearest thing today is a Daytona VM with a shell, or Bashkit, which is not a
real Linux process environment at all. An operator who wants the agent to build
on the machine they already trust, a CI runner, their dev box, a GPU host, has
no supported answer. Yolop's answer is its default mode and it is the whole
product.

## Model

Two orthogonal axes plus a negotiated capability set.

```text
                 containment:  none        native         isolated
                               (trusted)   (kernel)       (VM / interpreter)
target: host                   yolop        yolop           n/a
        (this machine)         default      --sandbox
        machine                ssh, agent   ssh + remote    n/a
        (registered box)       daemon       policy
        container              n/a          n/a            Docker
        managed                n/a          n/a            Daytona, E2B, ...
        vfs                    n/a          n/a            Bashkit
```

**Target** answers *where*: which filesystem and which process namespace the
tools address. **Containment** answers *what may be touched*: filesystem roots,
network, and the escalation path. "No sandbox" is `target: host,
containment: none`, an ordinary cell in the table, not a missing feature.

The two axes are not independent in the sense that every cell is reachable. They
are independent in the sense that a profile must state both, and neither may be
inferred from the other. A Daytona VM being `isolated` at the target level says
nothing about whether its egress is allowlisted; that is the containment field's
job.

### Naming

The Framework already named this. `Environment` in
`crates/host/src/workspace.rs` is "session execution resources", a workspace
head plus a type-keyed extension seam, and `EnvironmentBuilder::workspace_extension`
documents that seam for "process, container, or remote mount" providers that
must address the same head as the file tools. Compute was left as a future
extension; this proposal fills it in.

So there is no rename to argue about at the Framework level. The work is to
promote compute and containment from anonymous extensions to named members of
`Environment`, and to give the control plane a Sandbox resource that is the
durable projection of the same thing. Reserve "sandbox" for the security
property, which is what makes `containment: none` sayable without
contradiction.

The durable resource, generations, checkpoints, and reconciliation from
[Sandbox Abstraction](sandbox-abstraction.md) are unchanged.

### Environment profile

The sandbox profile in [Sandbox Abstraction](sandbox-abstraction.md) gains an
explicit containment block and an honest durability class:

```json
{
  "target": { "kind": "managed", "provider": "daytona" },
  "containment": {
    "level": "isolated",
    "filesystem": { "writable_roots": ["/home/daytona/workspace"] },
    "network": { "mode": "allowlist", "allowed_hosts": ["crates.io"] },
    "escalation": "approval"
  },
  "durability": "checkpointed",
  "lifecycle": { "...": "unchanged" },
  "bootstrap": { "...": "unchanged" }
}
```

`durability` is declared, never assumed: `checkpointed` (Everruns owns a
portable workspace checkpoint, eligible for durable-agent recovery),
`provider_snapshot` (fast restore only), `none` (a real machine; loss is loss).
An environment whose durability is `none` is not offered as a durable-agent
backend. It is not given a fake checkpoint either.

`escalation` says who may widen containment mid-session: `never`, `approval`
(human in the loop, Yolop's existing gate), or `auto` for trusted operator
setups.

### Capabilities stay negotiated

Extend the capability set already proposed with the containment facts the model
and the UI need: kernel-enforced boundary, network policy actually enforced,
native process execution, package installation, PTY, ports. Tools and UI are
assembled from this set. An unsupported operation is absent, never emulated.
Bashkit advertising "no native binaries" is the load-bearing example: it must
not look like a Linux shell that happens to be failing.

## Model-facing surface

One toolset for every cell of the table: `bash`, `read_file`, `write_file`,
`edit_file`, `glob`, `grep`. Provider-prefixed families
(`daytona_*`, `e2b_*`, `sandbox_*`) leave the agent-execution path and survive
only as advanced or operations tooling for workflows that genuinely manage
several environments as data.

Environment facts reach the model as live context, not prompt text. Yolop
already does this with `<environment_context>`, reporting effective mode and
network access while keeping the stable prompt free of live values. Everruns
adopts the same block: target, containment, network, writable roots,
capabilities, durability. One `coding` harness then replaces three, because the
only thing that differed between them was the environment.

## Letting the agent choose

The request is that an agent be able to select no sandbox, Bashkit, or Daytona.
Three levels, in increasing order of what the model itself decides:

**L1, declarative.** The agent version pins one environment profile. Already the
plan in [Sandbox Abstraction](sandbox-abstraction.md). Sufficient for most
deployments and the only level required for a first release.

**L2, a set with a switch.** The agent version declares *named* environments and
a default:

```json
{
  "environments": {
    "default": "scratch",
    "scratch": { "target": { "kind": "vfs", "provider": "bashkit" } },
    "build":   { "target": { "kind": "managed", "provider": "daytona" } },
    "here":    { "target": { "kind": "host" }, "containment": { "level": "native" } }
  }
}
```

The model gets one control tool, `use_environment { name }`, whose argument is
constrained to that map. It never passes provider configuration, an image, a
resource id, or a containment relaxation. The dangerous version of "the agent
picks its sandbox" is the model authoring the profile; the useful version is the
model choosing among profiles a human already approved. Only the second is
proposed.

**L3, escalation.** Within the current environment, request more access:
Yolop's `require_escalated` path, gated by human approval, grantable once or for
the session, with a sandbox-scoped grant never implying a full-access grant.
Already implemented in Yolop (`src/sandbox_approval.rs`) and worth lifting to
the platform rather than reinventing.

Invariant across all three: the model may pick from a preapproved set or narrow
its own access. Widening requires configuration or a human.

### Switching is a filesystem event

Each environment owns a working filesystem, so a switch has to say what happened
to the files. Exactly three modes, chosen by the profile, reported in the tool
result:

- `fresh`: the new environment starts bootstrapped and empty. Default.
- `carry`: export a portable workspace checkpoint from the old environment and
  import it into the new one. Requires both to advertise portable checkpoints,
  and is bounded by size.
- `attach`: both bind the same durable volume or mount. Same provider family
  only.

The result also states what did not survive: processes, servers, PTYs,
interpreter state, RAM. This is the same honesty the physical-loss recovery
event already owes the agent.

A switch changes the advertised capability set, so it changes the tool schemas.
It must land on a turn boundary with a schema refresh, never mid-batch.

## The machine target

A registered machine is a first-class target: a developer box, a CI runner, a
GPU host, or localhost for an embedded host like Yolop. Transport is SSH or a
small Everruns agent daemon; the credential is an ordinary connection record
with an owner, exactly like a Daytona API key.

Against the driver contract from [Sandbox Abstraction](sandbox-abstraction.md),
a machine is unremarkable: `WorkingFileSystem` over SFTP or the daemon,
`SandboxCompute` over an exec channel. What differs is honesty about
capabilities. Provision and delete are refused or become connect and disconnect,
because Everruns does not own the hardware. Durability is `none` unless the
operator opts into an rsync-style portable export. Native processes, packages,
PTY, and ports are all available, which is exactly why people want it.

`host` is the degenerate case of `machine`: same contract, in-process transport,
already half-built. `RealDiskFileStore` in `crates/host/src/real_disk.rs` is the
working filesystem, and the missing half is a compute implementation plus the
containment providers below.

## Yolop

Yolop is this proposal's `target: host` row, already shipped and further along
on containment than the platform is. Its
[sandboxing spec](https://github.com/everruns/yolop/blob/main/knowledge/specs/sandboxing.md)
anticipates the convergence explicitly: a provider boundary rather than
OS-specific policy in the tool, and a sketched `SandboxSession` extension for
providers with virtual filesystems, snapshots, or remote lifecycle.

The proposal is a trade in both directions.

**Everruns takes Yolop's containment layer.** Seatbelt and Landlock providers
move into a crate both consume, most likely under `everruns-host`, and become
the implementation of `containment.level = "native"` for the host and machine
targets. Everruns has nothing comparable today; Yolop has it tested on both
platforms in CI.

**Yolop takes Everruns' target layer.** Yolop's `SandboxProvider` keeps
answering "what may this process touch". A separate, optional target selection
answers "where does it run", implemented as the `SandboxCompute` and
`WorkingFileSystem` pair rather than the speculative `SandboxSession` in its
spec. A `yolop --env daytona` or `--env bashkit` then needs no new tool schemas,
because Yolop's `bash` and structured file tools already are the unified
surface this proposal wants everywhere.

Two Yolop invariants must survive the move, and both are already stated in its
spec: model input may never choose a host executable or silently widen mounts,
and startup fails closed when a required OS primitive is unavailable rather than
falling back to an unsandboxed host.

## Framework API

Proposed signatures, not implemented. They extend the existing `Environment`
seam rather than adding a parallel one, and follow the promotion rule in
[Application API Boundaries](../framework/application-api.md): `everruns-host`
owns the traits, `everruns` re-exports the value types an application composes.

### No sandbox, on this machine

The Yolop and coding-CLI position. Containment is stated, never defaulted,
because an omitted containment field that silently means `none` is how a
trusted-operator default becomes an accident.

```rust
use everruns::{Agent, Compute, Containment, Environment, InMemoryEngine};

let environment = Environment::builder()
    .workspace(head)                    // existing: WorkspaceHead
    .compute(Compute::host())           // this machine, in-process
    .containment(Containment::none())   // explicit
    .build()?;

let session = engine.create(agent).environment(environment).start().await?;
```

### Same machine, kernel containment

`Containment::native()` is Yolop's Seatbelt and Landlock policy, moved into the
shared crate. It fails closed: if the OS primitive is unavailable, `build()`
returns an error rather than degrading to the host.

```rust
let environment = Environment::builder()
    .workspace(head)
    .compute(Compute::host())
    .containment(
        Containment::native()
            .writable_root(head.path()?)
            .network(Network::Deny)
            .escalation(Escalation::Approval(approval_gate)),
    )
    .build()?;
```

### Bashkit

Bashkit brings its own filesystem and its own boundary, so it supplies both
halves. Asking for weaker containment than a target enforces is a build error,
not a silent upgrade.

```rust
let environment = Environment::builder()
    .compute(Compute::bashkit())        // provides the working filesystem too
    .build()?;

assert_eq!(environment.containment().level(), ContainmentLevel::Isolated);
```

### Daytona

```rust
let environment = Environment::builder()
    .compute(Compute::provider("daytona", daytona::Compute::small()
        .snapshot("everruns-rust")
        .workspace_path("/home/daytona/workspace")))
    .containment(Containment::isolated()
        .network(Network::allowlist(["crates.io", "static.crates.io"])))
    .durability(Durability::Checkpointed)
    .build()?;
```

### A registered machine

```rust
let environment = Environment::builder()
    .compute(Compute::machine(MachineTarget::ssh("build-01.internal")
        .connection(connection_id)      // credentials live in the connection
        .workspace_path("/srv/agent")))
    .containment(Containment::none())
    .build()?;

// Refused: the machine target advertises no portable checkpoint.
assert!(matches!(
    environment.durability(),
    Durability::None,
));
```

### Capabilities are asked, never assumed

```rust
let caps = session.environment().capabilities();
if !caps.native_processes {
    // Bashkit: `cargo build` will not work here. Say so, or switch.
}
```

### Named environments and the agent's choice

L2 from above. The set is authored by the application; the model receives one
control tool whose argument is constrained to these names.

```rust
let agent = Agent::builder()
    .instructions("You are an expert software developer.")
    .model(Model::openai("gpt-5.2"))
    .environments([
        ("scratch", scratch_environment),   // bashkit, cheap, no native binaries
        ("build",   daytona_environment),   // real Linux, network allowlist
    ])
    .default_environment("scratch")
    .environment_switching(Switching::Allowed { workspace: Transfer::Carry })
    .build()?;
```

From application code the same switch is explicit and reports what was lost:

```rust
let outcome = session.use_environment("build").carry_workspace().await?;
println!("{} files carried, {} lost", outcome.files_carried, outcome.processes_lost);
```

### Implementing a target

Two small traits, mirroring the driver contract in
[Sandbox Abstraction](sandbox-abstraction.md). A provider implements what it
has; absent capabilities are absent, not stubbed.

```rust
#[async_trait]
pub trait Compute: Send + Sync {
    fn capabilities(&self) -> ComputeCapabilities;
    async fn connect(&self, head: &WorkspaceHead) -> Result<ComputeHandle, ComputeError>;
}

#[async_trait]
pub trait ComputeSession: Send + Sync {
    async fn exec(&self, request: ExecRequest, sink: OutputSink) -> Result<ExecResult, ComputeError>;
    async fn cancel(&self, execution_id: &str) -> Result<(), ComputeError>;
}
```

`Containment` stays a separate, narrow trait so a target and a policy compose
independently: it maps a policy plus a resolved workspace root to a launch
decision, which is exactly Yolop's existing `SandboxProvider::command`
generalized past the local process case.

## Control-plane API

Today the whole surface is `GET`/`POST /v1/sessions/{session_id}/sandbox`
(`crates/server/src/api/session_sandbox.rs`), whose response mixes
configuration, lifecycle, and provider identity into one flat body with
`configured` and `exists` booleans. Proposed shapes follow
[API Conventions](../execution/api-conventions.md), including `allowed_actions`
computed from current state.

### Environments are agent configuration

```http
POST /v1/agents
```

```json
{
  "name": "coding",
  "harness": "coding",
  "environments": {
    "default": "scratch",
    "switching": { "allowed": true, "workspace": "carry" },
    "profiles": {
      "scratch": {
        "target": { "kind": "vfs", "provider": "bashkit" },
        "containment": { "level": "isolated", "network": { "mode": "deny" } },
        "durability": "checkpointed"
      },
      "build": {
        "target": { "kind": "managed", "provider": "daytona",
                    "options": { "size": "small", "snapshot": "everruns-rust" } },
        "containment": { "level": "isolated",
                         "network": { "mode": "allowlist", "allowed_hosts": ["crates.io"] } },
        "durability": "checkpointed",
        "lifecycle": { "idle_after_seconds": 180, "idle_action": "checkpoint_and_stop" }
      },
      "build-01": {
        "target": { "kind": "machine", "connection_id": "conn_...",
                    "options": { "workspace_path": "/srv/agent" } },
        "containment": { "level": "none", "escalation": "never" },
        "durability": "none"
      }
    }
  }
}
```

Validation is where the honesty is enforced: a `durability: none` profile on an
agent that requires durable recovery is a `422`, and a containment level a
target cannot enforce is a `422`, both with the `retry` error rel and a `hint`
naming the offending field.

### Session environment

```http
GET /v1/sessions/{session_id}/environment
```

```json
{
  "self_url": "https://api.example/v1/sessions/session_.../environment",
  "name": "build",
  "environment_id": "env_...",
  "target": { "kind": "managed", "provider": "daytona" },
  "containment": { "level": "isolated", "network": { "mode": "allowlist" } },
  "durability": "checkpointed",
  "capabilities": {
    "native_processes": true, "packages": true, "pty": true,
    "ports": true, "portable_checkpoint": true, "network_enforced": true
  },
  "desired_state": "ready",
  "observed_state": "ready",
  "generation": 17,
  "current_checkpoint_id": "sbxcp_...",
  "last_activity_at": "2026-09-09T10:31:02Z",
  "available": ["scratch", "build"],
  "allowed_actions": [
    { "rel": "pause",  "method": "POST", "operation_id": "manage_session_environment",
      "href": ".../environment", "hint": "Checkpoint and stop the instance." },
    { "rel": "delete", "method": "POST", "operation_id": "manage_session_environment",
      "href": ".../environment", "hint": "Discard the instance and its working filesystem." },
    { "rel": "switch", "method": "POST", "operation_id": "switch_session_environment",
      "href": ".../environment/switch", "schema_ref": "#/components/schemas/SwitchEnvironmentRequest",
      "hint": "Move this session to another profile the agent declares." }
  ]
}
```

`switch` is a new entry in the closed `rel` vocabulary and therefore a spec
change to [API Conventions](../execution/api-conventions.md), to be made
deliberately rather than by adding a one-off rel. `pause`, `resume`, and
`delete` reuse existing rels.

```http
POST /v1/sessions/{session_id}/environment/switch
```

```json
{ "to": "build", "workspace": "carry" }
```

```json
{
  "from": "scratch",
  "to": "build",
  "generation": 18,
  "workspace": { "mode": "carry", "files": 214, "bytes": 8134221, "revision": "wsr_..." },
  "lost": ["background_processes", "shell_state"],
  "observed_state": "ready"
}
```

`workspace: "carry"` against a target that advertises no portable checkpoint is
a `409` with a `retry` action hinting `fresh`, never a silent `fresh`.

### Targets this deployment can actually offer

The UI cannot render an honest picker from provider names alone.

```http
GET /v1/environment-targets
```

```json
{
  "items": [
    { "kind": "vfs", "provider": "bashkit", "available": true,
      "capabilities": { "native_processes": false, "packages": false, "pty": false,
                        "ports": false, "portable_checkpoint": true },
      "containment_levels": ["isolated"] },
    { "kind": "managed", "provider": "daytona", "available": true,
      "requires_connection": true,
      "capabilities": { "native_processes": true, "packages": true, "pty": true,
                        "ports": true, "portable_checkpoint": true },
      "containment_levels": ["isolated"] },
    { "kind": "host", "available": false,
      "reason": "host execution is disabled for this deployment",
      "containment_levels": ["none", "native"] }
  ]
}
```

### Escalation

An in-session request to widen containment surfaces as an event and is resolved
over the API, which is Yolop's approval gate with an HTTP front end.

```json
{ "type": "environment.escalation_requested", "escalation_id": "esc_...",
  "command": "cargo publish", "requested": { "network": { "mode": "allow" } },
  "reason": "publishing requires crates.io access" }
```

```http
POST /v1/sessions/{session_id}/environment/escalations/{escalation_id}
{ "decision": "approve_once" }
```

A `scope: session` grant of one level never implies a higher one, matching
Yolop's rule that a sandbox-scoped grant does not license full access later.

### Machines

```http
POST /v1/machines
{ "name": "build-01", "transport": "ssh", "address": "build-01.internal",
  "connection_id": "conn_..." }
```

Credentials stay in the connection record. A machine row carries no token, and
the profile references it by id, keeping the "no bearer credential in sandbox
rows, checkpoints, events, or logs" criterion from
[Sandbox Abstraction](sandbox-abstraction.md) intact.

### Events

`environment.switched`, `environment.instance_lost`, `environment.recovered`,
and `environment.escalation_requested` join the session event stream, so a UI
and a durable agent learn about a replaced incarnation the same way.

## What this removes

- `coding-container`, `coding-daytona`, `coding-session-sandbox` collapse into
  one `coding` harness plus environment profiles. Three duplicated prompts
  become one.
- `daytona_*`, `e2b_*`, `sandbox_*` leave the default agent toolset.
- `docker_container` is deleted, as
  [Sandbox Abstraction](sandbox-abstraction.md) already decided.
- Prompt text describing lifecycle, tool names, and idle timeouts, replaced by
  the live environment context block.

## Phasing

This sequences alongside the existing migration plan rather than restarting it.

**P0, containment field and capability catalog.** Add the containment block,
the durability class, and the containment capabilities to the profile; promote
compute and containment to named members of the Framework's existing
`Environment`; add `GET /v1/environment-targets`. No new providers. Exit: a
profile can express `host + none` and `daytona + isolated + allowlist`, and
validation rejects a durable-agent agent pinned to `durability: none`.

**P1, host target.** Compute implementation for the in-process host, joined to
the existing `RealDiskFileStore`. Containment providers extracted from Yolop
into the shared crate and wired to `containment.level`. Exit: an Everruns
session runs `bash` on the worker host under Landlock, and Yolop builds against
the shared containment crate with no behavior change.

**P2, environment sets.** Named environments, `use_environment`, switch modes,
schema refresh on switch, escalation gate lifted from Yolop. Exit: one session
starts in Bashkit, switches to Daytona for a build carrying its workspace, and
the transcript states exactly what was lost.

**P3, machine target and consolidation.** SSH or daemon transport, then port
E2B, Deno, Sprites, and container behind the driver contract as already planned.

P1 and P2 are independent of the Daytona durability work in
[Sandbox Abstraction](sandbox-abstraction.md) Phase 2 and can run beside it.

## Risks

- Capability-negotiated tools mean the toolset changes mid-session on a switch.
  Providers cache schemas; the turn-boundary rule is load-bearing, not a detail.
- Extracting Yolop's containment providers couples two release trains. The crate
  boundary must be small enough that Yolop can pin a published version, as it
  already does for Tuika and `everruns-host`.
- A machine target invites treating someone's laptop as durable agent
  infrastructure. The `durability: none` declaration must be enforced at
  validation time, not documented as a caveat.
- `carry` across targets is a full workspace transfer. Size bounds and cache
  exclusions decide whether it is usable or a trap.

## Open questions

1. The Framework already calls this `Environment`. Does the control plane's
   Sandbox resource take the same name, so one word spans both surfaces, or does
   `/v1/sessions/{id}/sandbox` keep its name with a containment field inside?
   Aligning is clearer and touches routes, tables, and UI.
2. Is L2 wanted in the first release, or is config-only selection (L1) enough
   until a workflow demands the switch?
3. Should the platform's host target require kernel containment by default, or
   is `host` a trusted-operator position where `none` is the sane default and
   Yolop's `--sandbox` opt-in is the model?
4. Where does the shared containment crate live: inside `everruns-host`, or its
   own publishable crate that both repositories depend on?
5. `switch` is a new entry in a deliberately closed `rel` vocabulary. Accept the
   spec change, or model a switch as delete plus create and lose the single
   atomic action?
