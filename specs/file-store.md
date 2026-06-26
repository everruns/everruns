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

This is the platform-level session filesystem seam. `PlatformDefinition`
carries a `SessionFileSystemFactory`, and runtime/server hosts resolve a live
`SessionFileSystem` from host dependencies such as in-memory state, a storage
backend, or a root directory. It is still compatible with the mount-overlay
resolver direction, which can compose mounts on top of a base filesystem.

## Background

The first non-server embedder of `everruns-runtime` (the `examples/coding-cli`
TUI in PR #1839) revealed a real seam: `AgentInstructionsCapability` and
`SkillsCapability` read project context (`AGENTS.md`,
`.agents/skills/...`) from the session VFS via
`SystemPromptContext.file_store`. That works inside the server (the session
VFS *is* the workspace) but breaks in an embedded coding-CLI where the
workspace is a real directory on disk.

The pluggable seam is `PlatformDefinition.session_file_system_factory`, so
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

### Traits

The public surface is two core traits:

- `everruns_core::traits::SessionFileSystem` — the read/write contract every
  filesystem-aware capability calls.
- `everruns_core::traits::SessionFileSystemFactory` — resolves the platform's
  chosen filesystem from host-provided dependencies.

See `crates/core/src/traits.rs` for the full method signatures and doc
comments. The filesystem trait shape is intentionally small: an implementation
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
not change. The model-facing namespace is a stable `/workspace` even for a
host-rooted backend; host-absolute display is opt-in rendering, not addressing.

Underneath, the workspace backend normalizes paths and maps to the host
filesystem through `everruns_core::workspace_paths::WorkspacePaths`. It is the
single normalizer/host-mapper the stores share, below the resolver.

- **Canonical form is workspace-relative.** The in-memory representation is
  `RelPath` — normalized, no `..`, no leading slash, no host prefix
  (`src/lib.rs`, `Cargo.toml`). The workspace root is the empty path.
- **The wire form stays leading-slash.** `RelPath::to_session_path()` produces
  the legacy `/src/lib.rs` form that `SessionFileSystem` methods and the DB
  store key on, so storage backends need no change.
- **`/workspace` is a display alias, not a namespace.** It is one accepted
  spelling on input and the default display prefix on output — never a second
  addressing scheme.

`WorkspacePaths` binds an optional host root (set for real-disk stores and
shells; absent for the pure VFS) shared behind a handle, so an embedder's
worktree switch via `set_host_root` propagates to every consumer that took its
`WorkspacePaths` from the same store. `spawn_cwd()` validates the root exists and
fails clearly (`workspace directory does not exist: …`) rather than letting a
spawn surface an opaque `No such file or directory`.

### Path Namespace

`parse_input` accepts every spelling below and normalizes to the same canonical
path. The store's wire form is **absolute, forward-slash, leading-slash**.

| Input | Canonical (`RelPath`) | Wire form |
|-------|-----------------------|-----------|
| `"src/bar.txt"` | `src/bar.txt` | `/src/bar.txt` |
| `"/src/bar.txt"` | `src/bar.txt` | `/src/bar.txt` |
| `"/workspace/src/bar.txt"` | `src/bar.txt` | `/src/bar.txt` |
| host-absolute under root | `src/bar.txt` | `/src/bar.txt` |
| `"/workspace"`, `""`, `"/"` | (root) | `/` |

Implementations MUST route path handling through `WorkspacePaths` and MUST NOT
hand-roll prefix logic. The shared parser guarantees:

- The optional leading `/workspace` segment is equivalent to the root. Bare
  `workspace/...` (no leading slash) is **not** the alias, so a real top-level
  `workspace/` directory stays reachable.
- Trailing slashes are stripped (except for `/`).
- Path traversal is rejected: `..` anywhere returns an error regardless of input
  shape, and host mapping re-checks containment under the root.
- Backslashes are not separators and environment variables are not expanded.
  Paths are opaque slash-separated segments.

### Display Paths

`WorkspacePaths::to_display` is the single formatter. The display prefix is
configurable, not magical:

- Default in-memory and storage-backed implementations display `/workspace`,
  matching the server workspace model.
- `RealDiskFileStore` displays its canonical host root by default and accepts
  host-absolute paths under that root as aliases for the same canonical paths.
  This lets embedders show `/Users/alex/project/src/lib.rs` while preserving
  `/workspace/src/lib.rs` as a valid compatibility input. An embedder may set
  the prefix to `/workspace` (to show the cloud alias) or to empty (bare
  relative paths) without changing the addressing.

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
5. **`grep_files`** searches text files only. Implementations are free to
   skip binary content, oversized files, and explicitly excluded
   directories. `path_pattern` is a plain substring match against the
   canonical path (no glob expansion); `Some("")` matches every path.
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

Source: `crates/runtime/src/in_memory.rs`.

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

Source: `crates/runtime/src/real_disk.rs`.

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

> Capabilities that accept `path` arguments MUST resolve them through
> `ToolContext.workspace_paths()` / `SystemPromptContext.workspace_paths()`
> (which derive from the file store) — or through the store's own
> `SessionFileSystem::workspace_paths()`. They MUST NOT implement their own
> `/workspace` stripping, alias handling, or containment checks. This includes
> host-process tools: the shell resolves its working directory and translates
> command paths through the same `WorkspacePaths`, so it shares one namespace
> with the file tools.

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

let platform = PlatformDefinition::builder()
    .session_file_system_factory(Arc::new(
        RealDiskSessionFileSystemFactory::new(workspace_root),
    ))
    .build();
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

- `crates/runtime/examples/real_disk_agent_instructions.rs` — proves
  `AgentInstructionsCapability` reads `AGENTS.md` from a real-disk root.
- `crates/runtime/examples/real_disk_file_system_tools.rs` — proves the
  `file_system` capability tools (`read_file`, `write_file`,
  `list_directory`) operate against a real-disk root.

## Policy Decorators

Embedders that need to apply policy to LLM-driven writes can compose
`SessionFileSystem` decorators on top of a base store. Two are shipped in
`everruns-runtime`:

- `WriteBlocklistFileStore` — rejects `write_file`, `delete_file`,
  `create_directory`, `seed_initial_file`, and
  `write_file_if_content_matches` whose path contains any blocked directory
  component (`.git`, `node_modules`, `target`, …) at any depth. Reads,
  listings, stats, and greps pass through. The blocklist defaults to
  `DEFAULT_WRITE_BLOCKLIST` and is fully overridable via
  `WriteBlocklistFileStore::with_blocklist(inner, custom)`.
- `ApprovalGatingFileStore` — gates `write_file`, `delete_file`, and the
  inner write inside `write_file_if_content_matches` through an embedder
  supplied `FileApprovalGate`. The trait has two async methods,
  `approve_write(path, before, after)` and
  `approve_delete(path, recursive)`. Reads pass through; `create_directory`
  and `seed_initial_file` are intentionally not gated (the subsequent
  `write_file` inside the new directory triggers the prompt, and seed
  files are embedder-supplied, not LLM-driven). Writes always read the
  inner store's existing content first so the embedder can render a diff.

The intended composition (eliding boilerplate):

```rust,ignore
let disk: Arc<dyn SessionFileSystem> =
    Arc::new(RealDiskFileStore::new(&workspace_root)?);
let blocklisted: Arc<dyn SessionFileSystem> =
    Arc::new(WriteBlocklistFileStore::new(disk));
let gated: Arc<dyn SessionFileSystem> =
    Arc::new(ApprovalGatingFileStore::new(blocklisted, gate));
```

Reads short-circuit through both layers; only the destructive paths take
the policy decisions. Each layer holds `Arc<dyn SessionFileSystem>` rather
than a generic inner so decorator stacks compose without coherence
gymnastics.

Smell to acknowledge: this puts policy in the storage layer rather than the
tool layer. The trade-off is uniformity — every built-in capability that
calls `ToolContext.file_store` / `SystemPromptContext.file_store` (today:
`file_system`, `agent_instructions`, `skills`, `web_fetch`,
`tool_output_persistence`) picks the policy up for free. The alternative
(tool-layer policy) would require every capability to wire its own gate and
its own blocklist, which the [examples/coding-cli][cli] prototype
demonstrated is the wrong default.

[cli]: ../examples/coding-cli/

## Non-goals

- **Multi-tenant `RealDiskFileStore`.** A future variant could carve
  per-session subdirectories under the root and use `session_id` as part
  of the path. The CLI use case does not need it; the spec keeps the door
  open by accepting (and ignoring) `session_id` in the current
  implementation.
- **Mount-overlay resolver (Option B).** A separate follow-up. Once
  landed, `RealDiskFileStore` becomes one possible `MountSource::HostPath`
  on top of the resolver. The trait shape here does not need to change.
- **Filesystem permissions for `is_readonly`.** Today `is_readonly` is
  honored at the trait layer (in-memory tracking) but is not mapped to
  `0o444` on disk.
- **`bashkit_shell` over real disk.** Replacing the in-VFS bash tool with
  a real-disk variant is a separate security conversation about
  unsandboxed shell execution.

## Forward Compatibility with Mount Overlay (Option B)

When the mount-overlay resolver lands, the migration path is:

1. `RealDiskFileStore` keeps its current trait impl and stays the base
   backend.
2. A new `MountSource::HostPath { root: PathBuf, access: MountAccess }`
   variant is added.
3. The resolver composes mounts on top of a base `SessionFileSystem`.
   Embedders that already use `RealDiskSessionFileSystemFactory` continue to
   work; the resolver wraps the resolved filesystem.
4. Capabilities can declare host-path mounts (e.g. `~/.local/share/data`
   as `/data` read-only) through `mounts()` instead of needing a custom
   `SessionFileSystem` impl.

## Source Index

- `crates/core/src/mount_fs.rs` — `MountFs` (the mount + cwd resolver, EVE-660)
- `crates/core/src/workspace_paths.rs` — `WorkspacePaths`, `RelPath` (the
  backend normalizer / host-mapper under the resolver, EVE-660)
- `crates/core/src/traits.rs` — `SessionFileSystem` trait
  (`workspace_paths()`), `ToolContext::workspace_paths()`
- `crates/core/src/session_file.rs` — `SessionFile`, `FileInfo`,
  `FileStat`, `GrepMatch`, `InitialFile`
- `crates/runtime/src/backends.rs` — `RuntimeBackends`
- `crates/runtime/src/file_store_decorators.rs` — `WriteBlocklistFileStore`,
  `ApprovalGatingFileStore`, `FileApprovalGate`, `DEFAULT_WRITE_BLOCKLIST`
- `crates/runtime/src/in_memory.rs` — `InMemorySessionFileStore`
- `crates/runtime/src/real_disk.rs` — `RealDiskFileStore`
- `crates/runtime/examples/real_disk_agent_instructions.rs` — wiring
  example for `AgentInstructionsCapability`
- `crates/runtime/examples/real_disk_file_system_tools.rs` — wiring
  example for `file_system` capability tools
- `crates/server/src/storage/session_file_store.rs` — `DbSessionFileStore`
- `specs/workspace.md` — `/workspace` mount and session VFS
  semantics
- `specs/runtime.md` — `RuntimeBackends` and the embedder seam
- `specs/capabilities.md` — `ToolContext` / `SystemPromptContext` wiring
