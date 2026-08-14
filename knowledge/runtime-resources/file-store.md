---
type: Specification
title: "Session Filesystem Specification"
description: "Pluggable `SessionFileStore` backends."
tags:
  - everruns
  - runtime-resources
---
# Session Filesystem Specification

## Abstract

`SessionFileSystem` is the canonical seam through which built-in capabilities
read and write files. Implementations of this trait decide what "the
workspace" physically is: an in-memory map, a PostgreSQL table, a remote gRPC
adapter, a real host-filesystem directory, or a future object-store-backed
filesystem.

This spec defines the contract every implementation must honor and the
discipline new capabilities should follow when they need filesystem access.

## Status

This is the platform-level session filesystem seam. `HostComposition`
carries a `SessionFileSystemFactory`, and runtime/server hosts resolve a live
`SessionFileSystem` from host dependencies such as in-memory state, a storage
backend, or a root directory. It is still compatible with the mount-overlay
resolver direction, and host-backed runtime sessions can now compose a primary
workspace root with additional mounted roots in one model-facing namespace.

## Background

The first non-server embedder of the in-process host (the `examples/coding-cli`
TUI in PR #1839) revealed a real seam: `AgentInstructionsCapability` and
`SkillsCapability` read project context (`AGENTS.md`,
`.agents/skills/...`) from the session VFS via
`SystemPromptContext.file_store`. That works inside the server (the session
VFS *is* the workspace) but breaks in an embedded coding-CLI where the
workspace is a real directory on disk.

The pluggable seam is `HostComposition.session_file_system_factory`, so
embedders choose the session filesystem as part of the deployment surface.

## Decision Process

We evaluated four options before landing on the pluggable-store approach:

| Option | Description | Outcome |
|--------|-------------|---------|
| A. Pluggable `SessionFileSystem` at runtime builder | Single trait, embedder picks the impl. | Superseded by platform factories. |
| B. Mount-point overlay (`MountSource::HostPath`) | Compose mounts on top of a base filesystem via a resolver. | Eventual destination. Strict superset of the factory seam. |
| C. Split `SystemPromptContext` into `file_store` + `WorkspaceSource` | Separate sandbox writes from project-context reads. | Rejected. Parallel abstraction that B subsumes naturally via `MountAccess::ReadOnly`. |
| D. Per-capability factory (`AgentInstructionsCapability::with_loader(...)`) | Each capability re-solves the loader problem itself. | Rejected. Does not compose; doesn't scale to N capabilities. |

A unblocks every existing built-in capability (`file_system`,
`agent_instructions`, `skills`, `web_fetch`, `tool_output_persistence`) the
day the embedder plugs a real-disk filesystem.

## Contract

### Contracts

The public surface is split across the execution and host concerns:

- `everruns_core::session_files::SessionFileSystem` — the read/write contract every
  filesystem-aware capability calls.
- `everruns_host::SessionFileSystemFactory` — resolves the deployment's
  chosen filesystem from host-provided dependencies.

See `crates/core/src/session_files.rs` and
`crates/host/src/session_file_system_factory.rs` for the full method signatures
and doc comments. The filesystem trait shape is intentionally small: an implementation
supports `read_file`, `write_file`, `write_file_if_content_matches` (CAS),
`delete_file`, `list_directory`, `stat_file`, `grep_files`,
`create_directory`, and `seed_initial_file`.

### Unified Workspace Path Model (EVE-660)

The agent sees **one** filesystem, resolved by
`everruns_core::mount_fs::MountFs` — a `SessionFileSystem` decorator the runtime
wires around the workspace backend wherever a file store enters a `ToolContext`
or `SystemPromptContext`. `MountFs` owns:

- a **mount table** — named mount points, each backed by a `SessionFileSystem`,
  with a per-mount root in that backend's keyspace, dispatched by longest-prefix
  match; and
- a **current working directory** (default `/workspace`) — relative paths
  resolve against it, `.`/`..` collapse POSIX-style.

`/workspace` is therefore a **mount point and the default cwd**, not a magic
prefix re-implemented in every store. Today there is a single workspace backend,
so the table holds the root mount (`/` → backend, for legacy backend-native
paths such as `/AGENTS.md`, `/outputs/…`, `/.agents/skills/…`) and the
`/workspace` view of the same backend; `/workspace` wins by longest-prefix, so
`/workspace/foo` ≡ `/foo`. Splitting `/outputs`, `/.agents/skills`, or volumes
onto *different* backends later is `MountFs::with_mount(...)` — the resolver does
not change. Routing and presentation are separate: direct backends retain their
own display identity for host-side integrations, while `MountFs` owns the stable
agent-facing identity.

For host-filesystem sessions with multiple registered workspace roots, the
primary root keeps the same layout and relative-path behavior:

- `/workspace/...` and relative paths resolve to the primary host root.
- `/workspace/roots/<name>/...` resolves to additional root `<name>`.
- Additional roots are intentionally not addressable by bare relative paths.

`everruns_core::WorkspaceRootSet` is the shared root-set contract for embedders.
It canonicalizes roots at construction, rejects duplicates and overlapping host
directories, parses model-facing VFS paths, and exposes host-scope helpers for
host-side tools. Embedders that need host paths MUST use this resolver instead
of copying `/workspace` stripping or containment logic locally.

`MountFs` is the only path authority. It normalizes input against the cwd,
collapses `.`/`..`, and dispatches to the longest-matching mount; the chosen
backend then keys on the resulting **leading-slash session path** (`/src/lib.rs`).
There is no shared "path model" object that callers reach for — capabilities use
the `SessionFileSystem` methods (`read_file`, `write_file`, `grep_files`,
`display_path`, `resolve_path`), and `MountFs` resolves underneath. The
host-agnostic `/workspace`-alias normalization is `everruns_core::session_path`
(`to_session_path`, `to_display_path`); mapping the virtual namespace onto a real
directory is a backend detail (see *Host mapping*).

- **The wire form is the leading-slash session path.** `/src/lib.rs`,
  `/Cargo.toml`, `/` for the root — what `SessionFileSystem` methods and the DB
  store key on. VFS backends key on it directly; no separate canonical type is
  shared across crates.
- **`/workspace` is a mount + the default cwd, not a namespace.** It is one
  accepted spelling on input and the default display prefix on output. The root
  mount (`/`) means **any** path is addressable — the shell can read and write
  anywhere from `/`, with `/workspace` simply being where it starts.

### Path Namespace

Every spelling below resolves to the same backend session path. The wire form is
**absolute, forward-slash, leading-slash**.

| Input (cwd `/workspace`) | Backend session path |
|--------------------------|----------------------|
| `"src/bar.txt"` | `/src/bar.txt` |
| `"/src/bar.txt"` | `/src/bar.txt` |
| `"/workspace/src/bar.txt"` | `/src/bar.txt` |
| host-absolute under root (real-disk) | `/src/bar.txt` |
| `"/workspace"`, `""`, `"/"` | `/` |
| `"/workspace/roots/backend/src/lib.rs"` (multi-root) | `/workspace/roots/backend/src/lib.rs` routed to the `backend` root |

Guarantees:

- The optional leading `/workspace` segment is equivalent to the root. Bare
  `workspace/...` (no leading slash) is **not** the alias, so a real top-level
  `workspace/` directory stays reachable.
- Trailing slashes are stripped (except for `/`); `.`/`..` collapse, with a
  leading `..` clamped at root.
- Host-backed stores reject `..` traversal and symlinks and re-check containment
  under the root, so resolving outside `/workspace` still cannot escape the
  backend root (it just lands at `<root>/etc/...`, not the host `/etc/...`).
- In multi-root sessions, host-absolute paths outside every registered root are
  rejected. Host-absolute paths under a registered root are accepted as aliases
  for that root after canonicalization.
- Backslashes are not separators and environment variables are not expanded.

### Host mapping (real-disk backend)

`MountFs` is a pure *virtual* router with no host knowledge. The only place that
needs to translate the virtual namespace onto a real directory is the host-backed
store, so that logic is private to `everruns_host::RealDiskFileStore`
(`HostPathMap`), not a shared abstraction:

- It maps a session path to an absolute host path, accepts host-absolute inputs
  under the root as aliases for the same file, rejects `..`/symlink escapes, and
  re-checks containment.
- The root is shared behind a handle, so an embedder's worktree switch via
  `RealDiskFileStore::set_host_root` is seen by every clone of the real-disk
  store at once. Mounted agent-facing presentation stays at `/workspace`.
- `WorkspaceRootSet::set_primary_host_root` and `RealDiskFileStore::set_host_root`
  repoint only the primary root. Additional roots are fixed for the session
  lifetime.

### Display Paths

Each backend owns its `display_path`/`display_root`:

- VFS/storage-backed stores display the `/workspace` alias (the
  `session_path::to_display_path` default).
- `RealDiskFileStore` displays its canonical host root, and accepts host-absolute
  paths under that root as aliases — so embedders can show
  `/Users/alex/project/src/lib.rs` while `/workspace/src/lib.rs` stays a valid
  input.
- `MountFs` is the agent-facing presentation boundary. **By default** it displays
  the stable `/workspace` namespace for primary workspace paths, so a real-disk
  primary rooted at `/repo` shows `/workspace/file.rs` to the model even though
  the backend can still display `/repo/file.rs` when used directly by host-side
  integrations. Embedders can override this per the display policy below.
- For additional mounted roots, returned file paths include the stable mounted
  prefix, e.g. `/workspace/roots/backend/Cargo.toml`.

#### Display policy: routing is not presentation (`DisplayPolicy`)

`/workspace` plays two independent roles in `MountFs`, and they are kept
separate on purpose:

1. **Routing / cwd** — the model addresses files at `/workspace/...` and relative
   paths resolve there. This is a runtime mechanism, identical for every embedder,
   so it stays hardcoded.
2. **Presentation** — the path string shown to the model, emitted in narration,
   and persisted in output pointers. This is *policy*, selected by
   `everruns_core::DisplayPolicy` on `MountFs`:
   - `WorkspaceAlias` (**default**) — present primary paths under the
     host-agnostic `/workspace` alias regardless of the backend. Required for
     multi-tenant/server hosts: a mounted real-disk session must not leak the host
     checkout path (`/private/var/.../checkout/...`) into model-visible or
     persisted output (threat model TM-FS). This is what PR #2776 established.
   - `BackendNative` (`MountFs::with_backend_display()`) — delegate primary-path
     presentation to the backend's own `display_path`/`display_root`, exposing
     real host paths. For local, single-user embedders (e.g. the `yolop` coding
     CLI, originally PR #258) where the host *is* the user's machine, so real,
     clickable paths that match `bash pwd` are the intended output. Such embedders
     still need `MountFs` for routing (relative resolution, default cwd, extra
     mounts), so they cannot drop it — hence presentation is a seam, not a fork.

Rationale for the seam: #2776 correctly hid host paths for the server but baked
that policy into the shared `MountFs` mechanism, which reverted #258 for local
embedders. Keeping `WorkspaceAlias` as the default preserves the server's
security property with no server changes, while `BackendNative` lets a local
embedder opt back in. The runtime therefore *defaults* presentation rather than
*hardcoding* it.

### Model-facing path guidance (EVE-748)

The agent-facing `SessionFileSystem` owns the model-visible path identity in
capability system prompts and file-tool parameter schemas:

- `MountFs` teaches `/workspace` for both storage-backed and real-disk primary
  workspaces.
- Direct real-disk backends retain their canonical host display root for
  host-side integrations, not model-facing contexts.
- Named secondary mounts remain discoverable through their mounted namespace.

Host-absolute input aliases may remain accepted internally by real-disk stores,
but mounted model context must not advertise them. Storage keys, backing roots,
and routing paths stay internal and must not leak into prompt text or schemas.
`FileSystemCapability` applies this through a `FilePathPresentation` hook at
tool-definition assembly time.

### Model-visible path identity conformance (EVE-750)

Every surface that can enter model context or a transcript must agree on one
display identity owned by the active `SessionFileSystem`:

- assembled system prompt and filesystem capability instructions;
- tool results, errors, persisted `output_files` / `full_output` pointers, and
  distillation notes;
- paths returned by list, grep, stat, read, write, edit, and delete tools.

**Invariants**

- Mounted primary: the model-visible identity is `/workspace` across prompt
  guidance, tool results, and persisted references, regardless of whether the
  backing store is virtual or rooted at a real host directory.
- Named secondary mounts: paths use the configured mounted virtual namespace
  (e.g. `/workspace/roots/backend/...`) and do not leak backing-store locations.

**Compatibility inputs vs presentation**

Host-absolute paths under registered roots may remain accepted as compatibility
inputs, but they must be canonicalized before any path is returned, narrated, or
persisted. Input compatibility is not model-visible identity.

**Conformance**

Tests use `everruns_core::path_identity` helpers (`assert_model_visible_value`,
`assert_no_forbidden_prefixes`, `assert_tool_result_paths_conform`) and the
runtime integration suite in
`crates/host/tests/model_visible_path_identity_test.rs`. The harness
recursively scans serialized JSON for absolute path-like strings rather than
enumerating field names, so new model-visible fields cannot bypass the check.

### Encoding

File content is round-tripped through two encodings:

- `"text"` — UTF-8 plain text.
- `"base64"` — standard base64 of arbitrary bytes.

Implementations MUST preserve the encoding the caller wrote (a `text` write
must read back as `text`; a `base64` write must read back as `base64`).
Auto-detection between text and binary happens above this layer (see
`SessionFile::encode_content`).

### `session_id` Semantics

The `session_id` parameter exists on every method but its meaning is
backend-specific:

| Backend | Semantics |
|---------|-----------|
| `InMemorySessionFileStore` | True per-session isolation; the map key is `(session_id, path)`. |
| `DbSessionFileStore` (server) | True per-session isolation; rows are FK-bound to the session. |
| `RealDiskFileStore` | **Ignored.** The store is rooted at a single host directory per process. The CLI use case is "one workspace, one shell, one ercode session at a time"; multi-tenant per-session subdirectories are out of scope (see below). |

`RealDiskFileStore` accepts the `session_id` parameter to keep the trait
shape uniform; implementations are free to log or assert on it, but they
MUST NOT use it as a path component without an explicit decision (see
"Multi-tenant `RealDiskFileStore`" below).

### Compare-and-Set

`write_file_if_content_matches` is a CAS primitive used by `edit_file` and
other tools that need to reject stale writes. The default trait impl reads,
compares, then writes — which has a documented narrow race window.
Implementations backed by transactional storage SHOULD override with an
atomic update. `RealDiskFileStore` uses the default stat-then-rewrite path
with the race window called out in its doc comment; tightening to
`flock`-based atomic CAS is a future enhancement.

### Behavior

The following behaviors hold across all implementations:

1. **Auto-create parents.** `write_file("/a/b/c.txt", ...)` MUST create
   `/a` and `/a/b` as directories if they are missing.
2. **Readonly protection.** Writes and deletes to files marked
   `is_readonly = true` MUST fail. The `is_readonly` flag enters the store
   through `seed_initial_file(InitialFile { is_readonly: true, .. })` and
   is honored within the same process. The disk backend tracks readonly
   marks in an in-memory set and does *not* set filesystem permissions, so
   the protection applies to writes through the store API only; the host's
   `chmod`/process boundary is out of scope.
3. **Directory delete.** `delete_file` on a non-empty directory with
   `recursive = false` MUST fail (returns `Ok(false)`); with
   `recursive = true` it MUST remove all descendants. Deleting the root
   itself MUST fail with an error, not return `Ok(false)`.
4. **`stat_file` on root** returns a synthetic directory entry; the root
   always exists.
5. **`grep_files`** searches text files only. The content `pattern` uses Rust
   [`regex`](https://docs.rs/regex) syntax and is compiled once before scanning;
   an invalid pattern returns an explicit error. Implementations are free to
   skip binary content and explicitly excluded directories. All implementations
   cap content and path patterns at 1,000 bytes, compiled regex size at 512 KiB,
   skip files above 512 KiB, and reject scans exceeding 5 MiB in total.
   `path_pattern` filters canonical workspace paths using globs:
   `*` and `?` match within one path segment, `**` crosses directories, and
   bracket classes and brace alternation are supported. A basename-only glob
   such as `*.txt` matches at any depth. `/workspace` and supported host-absolute
   aliases are normalized before matching. Patterns without glob
   metacharacters retain the legacy substring behavior; `Some("")` matches
   every path. `grep_files_with_options` adds match-based pagination and up to
   20 numbered lines of context before and after each selected match. Adjacent
   or overlapping windows are merged into blocks whose lines carry explicit
   match markers; a line appears at most once per block. `offset` and `limit`
   count matches, never context lines. Context is gathered during the backend
   content scan, and returned text is capped at 64 KiB with byte and next-match
   metadata. Zero context preserves the flat `GrepMatch` behavior.
6. **`list_directory` ambiguity.** The trait currently returns an empty
   `Vec<FileInfo>` both when the path is missing and when it exists but is
   not a directory. Callers that need to distinguish "empty directory"
   from "no such path" should `stat_file` the path first. A future
   refinement may switch to `Option<Vec<FileInfo>>` or an explicit error
   for "not found"; both the in-memory and real-disk backends will move
   together if that happens.

## Implementations

### `InMemorySessionFileStore`

The default backend bundled with `InProcessRuntime`. Per-session isolation,
auto-detects text/binary, suitable for tests and embedded use without disk
state.

Source: `crates/host/src/in_memory.rs`.

### `RealDiskFileStore`

A real-disk backend rooted at a `PathBuf` provided by the embedder. Path
mapping per the namespace rules above; `session_id` is accepted but
intentionally ignored.

Implementation notes:

- The constructor canonicalizes the root, so symlink resolution happens
  once.
- All path inputs are normalized and joined under the root, then verified
  to still live under the root before any I/O. This is the only traversal
  defense; the backend never trusts the caller's input.
- `grep_files` uses the `ignore` crate's `WalkBuilder` so `.gitignore`,
  `.ignore`, hidden-file rules, and `.git/` exclusion apply automatically.
  Non-UTF-8 path components are skipped rather than returned via
  `to_string_lossy` so `GrepMatch.path` always round-trips back through
  `resolve`.
- `read_file` auto-detects text vs. binary using
  `SessionFile::is_text_content`; binary content is returned base64.
- `write_file` accepts either encoding and writes raw bytes, decoded
  appropriately.
- `delete_file(_, recursive: false)` on a non-empty directory returns
  `Ok(false)`. `delete_file(_, _, true)` on the workspace root itself
  returns an explicit `tool` error so accidental sweeps are loud rather
  than silent.
- `seed_initial_file` writes through `write_file` and records the
  `is_readonly` flag in an in-memory set so subsequent writes/deletes to
  the same path fail per the trait contract.
- File sizes are saturated to `i64::MAX` (the `SessionFile.size_bytes`
  field is `i64`). Files larger than 9 EiB are not realistically reachable
  through this code path.

Source: `crates/host/src/real_disk.rs`.

### Multi-root host filesystems

`everruns-host` exposes `multi_root_file_system(root_set)` and teaches
`RealDiskSessionFileSystemFactory` to read an optional
`WorkspaceRootSet` from `SessionFileSystemFactoryContext::workspace_roots()`.
When present, the factory builds a `MountFs` over one `RealDiskFileStore` per
registered root:

- primary root at `/workspace` plus legacy root paths;
- each additional root at `/workspace/roots/<name>`;
- grep without a path searches every mounted root and reports mounted paths for
  non-primary hits.

An empty additional list is equivalent to the previous single-root behavior.

### Factories

- `InMemorySessionFileSystemFactory` resolves the runtime in-memory filesystem.
- `RealDiskSessionFileSystemFactory` resolves a filesystem rooted at a host
  directory.
- `StorageSessionFileSystemFactory` resolves the server storage-backed
  filesystem from `StorageBackend`.

Future factories can resolve S3/object-store-backed filesystems without
changing capability code.

### Future implementations

Worker-side gRPC adapters and the server's storage filesystem continue to
implement the same trait. Adding a new backend (e.g. S3, in-memory zip) means
implementing the trait and a factory; no capability API changes are required
upstream.

## Capability Rule

> New capabilities that need to read or write files MUST go through
> `ToolContext.file_store` or `SystemPromptContext.file_store`. They MUST
> NOT call `std::fs`, `tokio::fs`, or other host-filesystem APIs
> directly — *except* when the capability's execution model is inherently
> a host process (e.g. the bash tool spawns a shell, which inherits the
> host filesystem regardless of which `SessionFileSystem` is plugged in).

> Capabilities that accept `path` arguments MUST resolve them through the
> `SessionFileSystem` (the `file_store`, a `MountFs` in production) — its
> `read_file`/`write_file`/`grep_files`/`display_path`/`resolve_path` methods —
> and MUST NOT implement their own `/workspace` stripping, alias handling, or
> containment checks. This includes host-process tools: the shell seeds its
> working directory from `SessionFileSystem::resolve_path` and hands every path
> straight to the store, so it shares one namespace with the file tools and the
> resolver routes both.

This rule keeps all existing and future capabilities aligned with the
pluggable seam. A capability that follows the rule works against the
in-memory VFS, the database-backed server store, and the real-disk store
without code changes.

Exceptions are limited to host-process tools (bash and equivalents) and
require a comment in the capability's source naming the constraint that
forced the bypass. Anything else goes through the `SessionFileSystem`.

## Wiring

Embedders normally configure the platform factory:

```rust
let workspace_root = std::env::current_dir()?;

let platform = HostComposition::builder()
    .session_file_system_factory(Arc::new(
        RealDiskSessionFileSystemFactory::new(workspace_root),
    ))
    .build();
```

For multi-root host sessions, pass the root set through the factory context:

```rust,ignore
let roots = WorkspaceRootSet::new(
    primary_root,
    [("backend".to_string(), backend_root)],
)?;

let runtime = InProcessRuntimeBuilder::new()
    .host_composition(platform)
    .session_file_system_factory_context(
        SessionFileSystemFactoryContext::new()
            .with_workspace_roots(Arc::new(roots)),
    )
    .build()
    .await?;
```

Factories may require host dependencies such as a database handle, virtual
mount registry, or workspace root. `InProcessRuntimeBuilder` accepts a
`SessionFileSystemFactoryContext` for these host-supplied values and passes it
to the platform factory before seeding initial files.

The runtime forwards the resolved filesystem into every `ToolContext` and
`SystemPromptContext` it constructs, so every capability picks up the backend
automatically.

Server HTTP/API session-file management is a control-plane surface and remains
wired through `WorkspaceFileService`. The platform factory selects the filesystem
used by runtime/tool execution, not a replacement for those management
endpoints.

See the runnable examples for the full wiring against a real
`InProcessRuntime`:

- `crates/host/examples/real_disk_agent_instructions.rs` — proves
  `AgentInstructionsCapability` reads `AGENTS.md` from a real-disk root.
- `crates/host/examples/real_disk_file_system_tools.rs` — proves the
  `file_system` capability tools (`read_file`, `write_file`,
  `list_directory`) operate against a real-disk root.

## Policy Decorators

`WorkspacePolicy` is the application-facing policy value. The in-process host
wraps the filesystem selected by the platform factory, so the same
policy applies to in-memory, host-disk, database, and third-party providers.
The policy owns portable `/workspace` access decisions; concrete providers
still own storage mapping, containment, symlink handling, and atomic I/O.

The safe default permits reads of ordinary workspace files and denies writes,
hidden paths, sensitive paths, and recursive deletes. Applications opt into
writes and protected paths with explicit scopes. The broad read-write
convenience retains component-level denies for common dependency and build
directories; custom policies can configure their own component restrictions.
Denies win, and composition is an intersection so adding a layer cannot broaden
access. See the source for the API contract rather than duplicating its methods
here.

`PolicyFileStore` performs the uniform enforcement. Trusted application seed
files bypass mutation policy while being installed; every later model-driven
read or mutation is checked. Directory listings and grep output are filtered so
denied path names, content, counts, and byte totals are not returned.

The older `WriteBlocklistFileStore` remains as a low-level compatibility
decorator. Its `DEFAULT_WRITE_BLOCKLIST` export is deprecated and retained only
for 0.17 source compatibility; new policy behavior does not depend on that
global list. New applications use `WorkspacePolicy` instead of importing a
concrete store or a blocklist constant.

The independent approval decorator remains available:

- `ApprovalGatingFileStore` — gates `write_file`, `delete_file`, and the
  inner write inside `write_file_if_content_matches` through an embedder
  supplied `FileApprovalGate`. The trait has two async methods,
  `approve_write(path, before, after)` and
  `approve_delete(path, recursive)`. Reads pass through; `create_directory`
  and `seed_initial_file` are intentionally not gated (the subsequent
  `write_file` inside the new directory triggers the prompt, and seed
  files are embedder-supplied, not LLM-driven). Writes always read the
  inner store's existing content first so the embedder can render a diff.

Enforcement sits at the filesystem seam so every built-in capability that
calls `ToolContext.file_store` / `SystemPromptContext.file_store` (today:
`file_system`, `agent_instructions`, `skills`, `web_fetch`,
`tool_output_persistence`) observes one policy rather than reimplementing path
rules at each tool boundary.

## Non-goals

- **Multi-tenant `RealDiskFileStore`.** A future variant could carve
  per-session subdirectories under the root and use `session_id` as part
  of the path. The CLI use case does not need it; the spec keeps the door
  open by accepting (and ignoring) `session_id` in the current
  implementation.
- **Filesystem permissions for `is_readonly`.** Today `is_readonly` is
  honored at the trait layer (in-memory tracking) but is not mapped to
  `0o444` on disk.
- **`bashkit_shell` over real disk.** Replacing the in-VFS bash tool with
  a real-disk variant is a separate security conversation about
  unsandboxed shell execution.

## Forward Compatibility

`MountFs` remains the general composition point. Future capability-owned
mounts such as `/outputs`, `/.agents/skills`, or read-only data volumes can
still be layered with `MountFs::with_mount(...)` without changing capability
APIs or the `SessionFileSystem` trait.

## Source Index

- `crates/core/src/mount_fs.rs` — `MountFs` (the mount + cwd resolver, the only
  path authority, EVE-660)
- `crates/core/src/session_path.rs` — host-agnostic `/workspace`-alias helpers
  (`to_session_path`, `to_display_path`)
- `crates/core/src/workspace_roots.rs` — `WorkspaceRootSet` and host-root
  resolver for multi-root host sessions
- `crates/core/src/workspace_policy.rs` — portable `WorkspacePolicy`
- `crates/host/src/real_disk.rs` — `RealDiskFileStore` + its private
  `HostPathMap` (virtual ⇄ host mapping; the only host-rooted backend)
- `crates/core/src/session_files.rs` — `SessionFileSystem` trait
  (`display_path`/`display_root`/`resolve_path`)
- `crates/core/src/session_file.rs` — `SessionFile`, `FileInfo`,
  `FileStat`, `GrepMatch`, `InitialFile`
- `crates/host/src/backends.rs` — `HostBackends`
- `crates/host/src/file_store_decorators.rs` — `PolicyFileStore`,
  `WriteBlocklistFileStore`, its deprecated compatibility constant,
  `ApprovalGatingFileStore`, `FileApprovalGate`
- `crates/host/src/in_memory.rs` — `InMemorySessionFileStore`
- `crates/host/src/real_disk.rs` — `RealDiskFileStore`
- `crates/host/examples/real_disk_agent_instructions.rs` — wiring
  example for `AgentInstructionsCapability`
- `crates/host/examples/real_disk_file_system_tools.rs` — wiring
  example for `file_system` capability tools
- `crates/server/src/storage/session_file_store.rs` — `DbSessionFileStore`
- `knowledge/runtime-resources/workspace.md` — `/workspace` mount and session VFS
  semantics
- `knowledge/foundations/runtime.md` — `HostBackends` and the embedder seam
- `knowledge/execution/capabilities.md` — `ToolContext` / `SystemPromptContext` wiring
