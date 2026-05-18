# FileStore Specification

## Abstract

`SessionFileStore` (and its runtime extension `RuntimeFileStore`) is the
canonical seam through which built-in capabilities read and write files.
Implementations of this trait decide what "the workspace" physically is: an
in-memory map, a PostgreSQL table, a remote gRPC adapter, or a real
host-filesystem directory.

This spec defines the contract every implementation must honor and the
discipline new capabilities should follow when they need filesystem access.

## Status

This is the **Option A** pluggable-store seam (single trait, embedder picks
the impl). It is a stepping stone toward the mount-overlay resolver
(Option B) which will compose mounts on top of a base `FileStore`. The work
here is preserved when B lands; see the forward-compatibility note at the
bottom of this document.

## Background

The first non-server embedder of `everruns-runtime` (the `examples/coding-cli`
TUI in PR #1839) revealed a real seam: `AgentInstructionsCapability` and
`SkillsCapability` read project context (`AGENTS.md`,
`.agents/skills/...`) from the session VFS via
`SystemPromptContext.file_store`. That works inside the server (the session
VFS *is* the workspace) but breaks in an embedded coding-CLI where the
workspace is a real directory on disk.

The pluggable seam already existed (`RuntimeBackends.file_store`); what was
missing was a real-disk implementation and a documented contract.

## Decision Process

We evaluated four options before landing on the pluggable-store approach:

| Option | Description | Outcome |
|--------|-------------|---------|
| A. Pluggable `SessionFileStore` at runtime builder | Single trait, embedder picks the impl. Already supported by `RuntimeBackends.file_store`. | **Chosen.** Unblocks `coding-cli` with no core API changes. |
| B. Mount-point overlay (`MountSource::HostPath`) | Compose mounts on top of a base `FileStore` via a resolver. | Eventual destination. Strict superset of A; A is preserved when B lands. |
| C. Split `SystemPromptContext` into `file_store` + `WorkspaceSource` | Separate sandbox writes from project-context reads. | Rejected. Parallel abstraction that B subsumes naturally via `MountAccess::ReadOnly`. |
| D. Per-capability factory (`AgentInstructionsCapability::with_loader(...)`) | Each capability re-solves the loader problem itself. | Rejected. Does not compose; doesn't scale to N capabilities. |

A unblocks every existing built-in capability (`file_system`,
`agent_instructions`, `skills`, `web_fetch`, `tool_output_persistence`) the
day the embedder plugs a real-disk `FileStore`.

## Contract

### Traits

The public surface is two traits:

- `everruns_core::traits::SessionFileStore` — the read/write contract every
  filesystem-aware capability calls.
- `everruns_runtime::RuntimeFileStore` — extends `SessionFileStore` with
  `seed_initial_file`, used during session bootstrap.

See `crates/core/src/traits.rs` and `crates/runtime/src/backends.rs` for the
full method signatures and doc comments. The trait shape is intentionally
small: an implementation supports `read_file`, `write_file`,
`write_file_if_content_matches` (CAS), `delete_file`, `list_directory`,
`stat_file`, `grep_files`, and `create_directory`.

### Path Namespace

`FileStore` paths are **absolute, forward-slash, leading-slash** strings.

| Input | Canonical form |
|-------|----------------|
| `"/foo/bar.txt"` | `/foo/bar.txt` |
| `"/workspace/foo/bar.txt"` | `/foo/bar.txt` |
| `"/workspace"` | `/` |
| `""` or `"/"` | `/` |
| `"foo.txt"` (no leading slash) | `/foo.txt` |

Implementations MUST:

- Treat the optional leading `/workspace` segment as equivalent to the root.
  The `/workspace` prefix exists because agents reason about a `/workspace`
  mount point (see `specs/session-filesystem.md`), but the store itself
  works in a flat per-session namespace.
- Strip trailing slashes (except for `/`).
- Reject path traversal: any path that, after normalization, would escape
  the store root MUST return an error. `..` segments anywhere in the path
  are treated as a traversal attempt regardless of input shape (absolute,
  relative, or `/workspace`-prefixed).
- Never interpret backslashes as separators or environment variables as
  expansions. Paths are opaque strings of slash-separated segments.

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

### Future implementations

Worker-side gRPC adapters and the server's `DbSessionFileStore` continue to
implement the same trait. Adding a new backend (e.g. S3, in-memory zip)
means implementing the trait; no API changes are required upstream.

## Capability Rule

> New capabilities that need to read or write files MUST go through
> `ToolContext.file_store` or `SystemPromptContext.file_store`. They MUST
> NOT call `std::fs`, `tokio::fs`, or other host-filesystem APIs
> directly — *except* when the capability's execution model is inherently
> a host process (e.g. the bash tool spawns a shell, which inherits the
> host filesystem regardless of which `FileStore` is plugged in).

This rule keeps all existing and future capabilities aligned with the
pluggable seam. A capability that follows the rule works against the
in-memory VFS, the database-backed server store, and the real-disk store
without code changes.

Exceptions are limited to host-process tools (bash and equivalents) and
require a comment in the capability's source naming the constraint that
forced the bypass. Anything else goes through the `FileStore`.

## Wiring

Embedders construct `RuntimeBackends` with their chosen `file_store`:

```rust
let workspace_root = std::env::current_dir()?;
let file_store = Arc::new(RealDiskFileStore::new(workspace_root)?);

let backends = RuntimeBackends {
    file_store,
    // ...
};
```

The runtime forwards this `file_store` into every `ToolContext` and
`SystemPromptContext` it constructs, so every capability picks up the new
backend automatically.

See the runnable examples for the full wiring against a real
`InProcessRuntime`:

- `crates/runtime/examples/real_disk_agent_instructions.rs` — proves
  `AgentInstructionsCapability` reads `AGENTS.md` from a real-disk root.
- `crates/runtime/examples/real_disk_file_system_tools.rs` — proves the
  `file_system` capability tools (`read_file`, `write_file`,
  `list_directory`) operate against a real-disk root.

## Non-goals

- **Policy decorators (write blocklists, approval gating, etc.).** These
  are deferred. Embedders that need them today can wrap the trait
  themselves; the foundational seam is enough to unblock the use cases on
  the table. Decorators can land alongside a concrete embedder that needs
  them.
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
- **`virtual_bash` over real disk.** Replacing the in-VFS bash tool with
  a real-disk variant is a separate security conversation about
  unsandboxed shell execution.

## Forward Compatibility with Mount Overlay (Option B)

When the mount-overlay resolver lands, the migration path is:

1. `RealDiskFileStore` keeps its current trait impl and stays the base
   backend.
2. A new `MountSource::HostPath { root: PathBuf, access: MountAccess }`
   variant is added.
3. The resolver composes mounts on top of a base `FileStore`. Embedders
   that already pass `RealDiskFileStore` as `file_store` continue to work;
   the resolver wraps it.
4. Capabilities can declare host-path mounts (e.g. `~/.local/share/data`
   as `/data` read-only) through `mounts()` instead of needing a custom
   `FileStore` impl.

## Source Index

- `crates/core/src/traits.rs` — `SessionFileStore` trait
- `crates/core/src/session_file.rs` — `SessionFile`, `FileInfo`,
  `FileStat`, `GrepMatch`, `InitialFile`
- `crates/runtime/src/backends.rs` — `RuntimeFileStore`, `RuntimeBackends`
- `crates/runtime/src/in_memory.rs` — `InMemorySessionFileStore`
- `crates/runtime/src/real_disk.rs` — `RealDiskFileStore`
- `crates/runtime/examples/real_disk_agent_instructions.rs` — wiring
  example for `AgentInstructionsCapability`
- `crates/runtime/examples/real_disk_file_system_tools.rs` — wiring
  example for `file_system` capability tools
- `crates/server/src/storage/session_file_store.rs` — `DbSessionFileStore`
- `specs/session-filesystem.md` — `/workspace` mount and session VFS
  semantics
- `specs/runtime.md` — `RuntimeBackends` and the embedder seam
- `specs/capabilities.md` — `ToolContext` / `SystemPromptContext` wiring
