---
title: Workspaces and Environments
description: Isolate writable project heads, bind them permanently to sessions, and reopen them safely.
---

An `Agent` describes behavior. A `Session` owns conversation continuity. An
`Environment` fixes the execution resources for that session, beginning with
one `WorkspaceHead`.

A `Workspace` is logical project lineage, not a directory alias. Each
`WorkspaceHead` is a stable, provider-owned mutable view of that lineage. All
heads present the same portable `/workspace` namespace even when a provider
implements them as Git worktrees, remote volumes, or another storage system.

## Isolated local Git heads

Enable the `local` feature to use the public Git-worktree provider:

```rust
use std::sync::Arc;
use everruns::{
    Agent, InMemoryEngine, LocalGitWorkspaceProvider, Model, Workspace, WorkspacePolicy,
};

# async fn example(repository: &std::path::Path, state: &std::path::Path)
# -> Result<(), Box<dyn std::error::Error>> {
let provider = Arc::new(LocalGitWorkspaceProvider::new(state)?);
let workspace = Workspace::open(provider, repository.to_string_lossy()).await?;
let head = workspace
    .head("feature")
    .from_revision("main")
    .create()
    .await?;
let agent = Agent::builder()
    .instructions("Work in the selected project head.")
    .model(Model::simulated("ready"))
    .workspace_policy(WorkspacePolicy::read_write())
    .build()?;
let engine = InMemoryEngine::new();
let session = engine.create(agent).workspace(head).start().await?;

assert!(session.workspace_head().is_some());
# Ok(())
# }
```

Head creation is isolated by default. The Framework rejects binding the same
isolated head to a second session. Opt into a shared mutable head with
`workspace.head("shared").shared().create()`. A shared real-disk head does not
become isolated: Framework compare-and-set writes report stale-content
conflicts within the host process, and provider status reports Git conflict and
dirty metadata. Coordinate other writers at the application or provider layer.

Use `head.fork("name").await` to create an isolated head from the current
checkpoint. `checkpoint`, `status`, `archive`, and `destroy` are explicit
lifecycle operations. Dropping a head, session, agent, workspace, or provider
never deletes a worktree or branch. The local provider's explicit `destroy`
removes the worktree and retains its Git branch. Archive blocks later reopen;
it does not revoke a filesystem handle already owned by a running session.

## Exact resume

`start()` persists the provider's credential-free opaque binding before the
session can execute. `Engine::resume` asks the recorded provider to reopen that
exact workspace and head. It returns a structured `ResumeError` when the
provider is missing, the head is unavailable, the binding is corrupt, or the
provider returns a different identity. It never substitutes an empty or
different head.

After a process restart, the Agent attached to a durable session created from
an explicit provider must register that provider with
`AgentBuilder::workspace_provider`. The provider used by a live Environment is
remembered automatically in that Agent snapshot. The default memory provider
and `AgentBuilder::workspace(path)` shorthand provider are registered by the
Framework itself.

## Workspace, roots, policy, and sandbox

These concepts are deliberately separate:

| Concept | Meaning |
| --- | --- |
| `Workspace` | Logical project or lineage |
| `WorkspaceHead` | One reopenable mutable view selected for a session |
| `/workspace` | Stable model-visible path presented by the head filesystem |
| Additional roots | Extra named mounts in `WorkspaceRootSet`; never heads or lineage |
| `WorkspacePolicy` | Portable read/write authorization composed over the selected filesystem |
| Sandbox | A process/compute isolation boundary; not provided by path policy alone |

The Environment carries its head plus an open type-keyed extension seam for
future compute or network resources. Providers implement the async
`WorkspaceProvider` trait directly; there is no backend enum or vendor switch.
Every head supplies the existing `SessionFileSystem`, so file tools, seeded
files, containment checks, mounts, and `WorkspacePolicy` remain one stack.
When a compute resource needs the selected filesystem, attach it with
`EnvironmentBuilder::workspace_extension`; its constructor receives the exact
head that Framework file tools will use.

For a simple application, `engine.create(agent)` is the concise path.
Its first `send` or `inspect` selects the default head automatically; optional
`session.start().await` selects it earlier without running a turn.
`AgentBuilder::workspace(path)` is shorthand for one explicitly shared local
directory across that Agent's sessions; it does not create isolated heads.
The shorthand is still a first-class shared head and its exact canonical path
binding is persisted for resume. Choose an Environment when isolation,
forking, or provider-specific lifecycle matters.
