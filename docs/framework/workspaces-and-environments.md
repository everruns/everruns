---
title: Workspaces and Environments
description: Isolate writable project heads, bind them to sessions, reopen them safely, and configure safe read and write scopes.
---

An `Agent` describes behavior. A `Session` owns conversation continuity. An
`Environment` fixes the execution resources for that session, beginning with
one `WorkspaceHead`.

A `Workspace` is logical project lineage, not a directory alias. Each
`WorkspaceHead` is a stable, backend-owned mutable view of that lineage. All
heads present the same portable `/workspace` namespace even when a backend
implements them as Git worktrees, remote volumes, or another storage system.

## Isolated local Git heads

Enable the `local` feature to use the public Git-worktree backend:

```rust
use std::sync::Arc;
use everruns::{
    Agent, Engine, LocalGitWorkspace, OpenAI, Workspace, WorkspacePolicy,
};

# async fn example(repository: &std::path::Path, state: &std::path::Path)
# -> Result<(), Box<dyn std::error::Error>> {
let backend = Arc::new(LocalGitWorkspace::new(state)?);
let workspace = Workspace::open(backend, repository.to_string_lossy()).await?;
let head = workspace
    .head("feature")
    .from_revision("main")
    .create()
    .await?;
let agent = Agent::builder()
    .instructions("Work in the selected project head.")
    .provider(OpenAI::from_env()?)
    .model("gpt-5.6-terra")
    .workspace_policy(WorkspacePolicy::read_write())
    .build()?;
let engine = Engine::new();
let session = engine.create(agent).workspace(head).start().await?;

assert!(session.workspace_head().is_some());
# Ok(())
# }
```

Head creation is isolated by default. The Framework rejects binding the same
isolated head to a second session. Opt into a shared mutable head with
`workspace.head("shared").shared().create()`. A shared real-disk head does not
become isolated: Framework compare-and-set writes report stale-content
conflicts within the host process, and backend status reports Git conflict and
dirty metadata. Coordinate other writers at the application or backend layer.

Use `head.fork("name").await` to create an isolated head from the current
checkpoint. `checkpoint`, `status`, `archive`, and `destroy` are explicit
lifecycle operations. Dropping a head, session, agent, workspace, or backend
never deletes a worktree or branch. The local backend's explicit `destroy`
removes the worktree and retains its Git branch. Archive blocks later reopen;
it does not revoke a filesystem handle already owned by a running session.

## Exact resume

`start()` persists the backend's credential-free opaque binding before the
session can execute. `Engine::resume` asks the recorded backend to reopen that
exact workspace and head. It returns a structured `ResumeError` when the
backend is missing, the head is unavailable, the binding is corrupt, or the
backend returns a different identity. It never substitutes an empty or
different head.

After a process restart, the Agent attached to a durable session created from
an explicit backend must register that backend with
`AgentBuilder::workspace_backend`. The backend used by a live Environment is
remembered automatically in that Agent snapshot. The default memory backend
and `AgentBuilder::workspace(path)` shorthand backend are registered by the
Framework itself.

## Compatibility window

Use `WorkspaceBackend`, `WorkspaceBackendId`, `LocalGitWorkspace`,
`AgentBuilder::workspace_backend`, and `WorkspaceHead::backend` in new code.
The provider-named types and methods remain as deprecated forwarding aliases.

Existing error matches keep their behavior during the deprecation window.
Framework and built-in backend paths continue to emit
`BuildError::DuplicateWorkspaceProvider`,
`ResumeError::WorkspaceProviderUnavailable`,
`SessionEnvironmentError::ProviderConflict`,
`WorkspaceError::ProviderUnavailable`, and `WorkspaceError::Provider`.
Their replacements are `BuildError::DuplicateWorkspaceBackend`,
`ResumeError::WorkspaceBackendUnavailable`,
`SessionEnvironmentError::BackendConflict`,
`WorkspaceError::BackendUnavailable`, and `WorkspaceError::Backend`. Match both
names while migrating. New `WorkspaceBackend` implementations should return the
backend-named `WorkspaceError` variants.

Persisted `WorkspaceBinding::provider_id`, SQLite columns, and existing
`workspace-provider` state directory names do not change in this migration.

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

The Environment carries its head plus an open type-keyed extension boundary for
future compute or network resources. Backends implement the async
`WorkspaceBackend` trait directly; there is no backend enum or vendor switch.
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
forking, or backend-specific lifecycle matters.

## Workspace security

`WorkspacePolicy` is the portable security boundary for files visible to an
in-process agent. Applications configure it through `everruns`; they do not
need `RealDiskFileStore`, `HostBackends`, or a runtime-owned blocklist.

```rust
use everruns::{Agent, OpenAI, WorkspacePolicy};

fn build(root: &std::path::Path) -> Result<Agent, Box<dyn std::error::Error>> {
    let policy = WorkspacePolicy::builder()
        .allow_read("/")
        .allow_write("generated")
        .deny_write("generated/locked")
        .allow_hidden(".github")
        .build()?;

    Ok(Agent::builder()
        .instructions("Work only inside the configured workspace.")
        .provider(OpenAI::from_env()?)
        .model("gpt-5.6-terra")
        .workspace(root)
        .workspace_policy(policy)
        .build()?)
}
```

### Defaults

`WorkspacePolicy::default()` and `WorkspacePolicy::read_only()` use the same
secure baseline:

| Operation | Default |
|---|---|
| Read ordinary workspace files | Allowed |
| Write, create, or delete | Denied |
| Read or write hidden paths | Denied |
| Read or write common credential paths | Denied |
| Read framework-managed `.agents` content | Allowed |
| Recursively delete a directory | Denied |

`WorkspacePolicy::read_write()` is an explicit opt-in to ordinary writes. It
does not expose additional hidden or sensitive paths and does not enable
recursive deletion. It also keeps common dependency and build directories such
as `node_modules` and `target` non-writable at every depth. For narrower access,
start with
`WorkspacePolicy::builder()`, which has no readable or writable scopes until
you add them.

A custom builder does not inherit the default `.agents` exception. If the
agent needs workspace-provided instructions or skills, add both a readable
scope and a narrow `allow_hidden(".agents")` opt-in.

Protected path names are defense in depth, not content-based secret scanning.
Keep credentials outside the mounted workspace, add explicit deny scopes for
application-specific secret locations, and never place credentials in `.agents`
content.

### Matching and precedence

Scopes are literal path prefixes, not globs, and compare ASCII letters without
case sensitivity so a deny cannot be bypassed on a case-insensitive backend.
`generated` therefore includes `generated/report.md` but not
`other/generated/report.md`.

- A deny scope always wins over an allow scope.
- `deny_write_component` rejects an exact directory or file name at every
  depth; `deny_write` rejects one rooted path subtree.
- Hidden paths need `allow_hidden` for the narrow path that should be visible.
- Common credential paths need the stronger `allow_sensitive` opt-in, which
  also permits hidden components inside that specific scope.
- `compose` is restrictive: every composed policy must allow the operation.
  A library can add constraints without accidentally broadening the
  application's policy.

Trusted starter files are installed before model access is enforced. This lets
an application seed a read-only file even under a non-writable policy. Later
reads, writes, and deletes of that file still go through the policy, so seeding
a hidden file does not automatically expose it.

### Paths and containment

Policy paths live in one portable workspace namespace. These spellings identify
the same file:

```text
src/lib.rs
/src/lib.rs
/workspace/src/lib.rs
```

Traversal (`..`), NUL bytes, and backslash-separated paths fail closed. Host
absolute paths are backend-specific and are not portable policy scopes; use
`/workspace/...` in application configuration and model instructions.

The policy layer controls visibility and mutation. The selected filesystem
backend remains responsible for mapping workspace paths to storage. The local
host backend canonicalizes its root, keeps resolved paths contained, and
rejects symlinks in existing path components before every operation. An
absolute path outside the configured root cannot expose that host file.

The policy governs capabilities that use Everruns' session filesystem. A
custom tool that calls `std::fs`, launches a shell, or uses another storage API
does not pass through this boundary. Apply equivalent restrictions to those
tools or run them in a sandbox.

### Symlinks and races

The built-in local backend rejects a symlink introduced after the workspace
was configured because it rechecks components on every operation. This blocks
normal traversal and symlink-swap attempts between operations.

It is not an OS sandbox. A malicious process running as the same operating
system user can race a final path check and filesystem syscall. If local
processes are mutually untrusted, use an isolated sandbox/filesystem backend
or operating-system isolation. Do not use `WorkspacePolicy` as a substitute for
that process boundary.

### Backend extension

The in-process host applies the policy after resolving the platform's
filesystem factory. In-memory, local-disk, database, and custom backends all
receive the same policy checks. Backend authors still own containment,
symlink-safe I/O, quotas, durability, and atomic update guarantees for their
storage system.

Directory listings and grep are enforced at the same boundary as direct reads.
Denied files are not opened by policy grep, and denied names, match counts, and
byte totals are not returned. Recursive deletes inspect descendants through the
backend before deletion, so opting into recursion does not override a deny or
protected descendant. Backends with mutable external state must still treat
that preflight-to-delete window as a race boundary.

`WorkspaceRootSet` additional roots are named mounts inside one selected head;
they are not independent heads and carry no fork/reopen lifecycle. Likewise,
`WorkspacePolicy` is path authorization, not a compute sandbox. The sections
above describe the identity and lifecycle model.
