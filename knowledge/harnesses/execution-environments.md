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

Call the resolved configuration an **Environment** and the running thing an
**environment instance**. Reserve "sandbox" for the security property. This is
what makes `sandbox: none` sayable without contradiction, and it matches the
vocabulary Claude Managed Agents and LangChain backends already use, cited in
[Sandbox Abstraction](sandbox-abstraction.md).

The durable resource, generations, checkpoints, and reconciliation from that
concept are unchanged; the tables and the manager are renamed, not redesigned.

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

**P0, vocabulary and containment field.** Rename the resource to Environment,
add the containment block, the durability class, and the containment
capabilities to the profile. No new providers. Exit: a profile can express
`host + none` and `daytona + isolated + allowlist`, and validation rejects a
durable-agent agent pinned to `durability: none`.

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

1. Rename to Environment, or keep "Sandbox" as the resource name with
   `containment.level = "none"` inside it? The rename is clearer and touches API
   routes, tables, and UI.
2. Is L2 wanted in the first release, or is config-only selection (L1) enough
   until a workflow demands the switch?
3. Should the platform's host target require kernel containment by default, or
   is `host` a trusted-operator position where `none` is the sane default and
   Yolop's `--sandbox` opt-in is the model?
4. Where does the shared containment crate live: inside `everruns-host`, or its
   own publishable crate that both repositories depend on?
