---
title: Workspace Security
description: Configure safe read and write scopes for in-process Everruns agents.
---

`WorkspacePolicy` is the portable security boundary for files visible to an
in-process agent. Applications configure it through `everruns`; they do not
need `RealDiskFileStore`, `HostBackends`, or a runtime-owned blocklist.

```rust
use everruns::{Agent, Model, WorkspacePolicy};

fn build(root: &std::path::Path) -> Result<Agent, Box<dyn std::error::Error>> {
    let policy = WorkspacePolicy::builder()
        .allow_read("/")
        .allow_write("generated")
        .deny_write("generated/locked")
        .allow_hidden(".github")
        .build()?;

    Ok(Agent::builder()
        .instructions("Work only inside the configured workspace.")
        .model(Model::simulated("ready"))
        .workspace(root)
        .workspace_policy(policy)
        .build()?)
}
```

## Defaults

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

## Matching and precedence

Scopes are literal path prefixes, not globs, and compare ASCII letters without
case sensitivity so a deny cannot be bypassed on a case-insensitive provider.
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

## Paths and containment

Policy paths live in one portable workspace namespace. These spellings identify
the same file:

```text
src/lib.rs
/src/lib.rs
/workspace/src/lib.rs
```

Traversal (`..`), NUL bytes, and backslash-separated paths fail closed. Host
absolute paths are provider-specific and are not portable policy scopes; use
`/workspace/...` in application configuration and model instructions.

The policy layer controls visibility and mutation. The selected filesystem
provider remains responsible for mapping workspace paths to storage. The local
host provider canonicalizes its root, keeps resolved paths contained, and
rejects symlinks in existing path components before every operation. An
absolute path outside the configured root cannot expose that host file.

The policy governs capabilities that use Everruns' session filesystem. A
custom tool that calls `std::fs`, launches a shell, or uses another storage API
does not pass through this boundary. Apply equivalent restrictions to those
tools or run them in a sandbox.

## Symlinks and races

The built-in local provider rejects a symlink introduced after the workspace
was configured because it rechecks components on every operation. This blocks
normal traversal and symlink-swap attempts between operations.

It is not an OS sandbox. A malicious process running as the same operating
system user can race a final path check and filesystem syscall. If local
processes are mutually untrusted, use an isolated sandbox/filesystem provider
or operating-system isolation. Do not use `WorkspacePolicy` as a substitute for
that process boundary.

## Provider extension

The in-process host applies the policy after resolving the platform's
filesystem factory. In-memory, local-disk, database, and custom providers all
receive the same policy checks. Provider authors still own containment,
symlink-safe I/O, quotas, durability, and atomic update guarantees for their
storage system.

Directory listings and grep are enforced at the same boundary as direct reads.
Denied files are not opened by policy grep, and denied names, match counts, and
byte totals are not returned. Recursive deletes inspect descendants through the
provider before deletion, so opting into recursion does not override a deny or
protected descendant. Providers with mutable external state must still treat
that preflight-to-delete window as a race boundary.

`WorkspaceRootSet` additional roots are named mounts inside one selected head;
they are not independent heads and carry no fork/reopen lifecycle. Likewise,
`WorkspacePolicy` is path authorization, not a compute sandbox. See
[Workspaces and Environments](/framework/workspaces-and-environments/) for the
identity and lifecycle model.
