# Stateful Bash (Session Shell State)

> **Status: PROPOSED** — design intent for making the `virtual_bash` capability
> behave like a persistent REPL across tool calls within a session.

## Why

Today every `bash` tool call builds a fresh `Bash` instance, runs the script, and
drops it (`crates/core/src/capabilities/virtual_bash.rs`, `execute_with_context`).
The **filesystem already persists** — bashkit's VFS is bridged to the session file
store via `SessionFileSystemAdapter`, so files written in one call are visible to the
next (see `specs/bashkit-requirements.md`). What is lost between calls is **shell
state**: working directory, exported/shell variables, functions, aliases, traps, and
the cumulative session counters.

This makes the tool surprising: `cd /workspace/foo` then `ls` in a later call starts
back at `/workspace`; `export TOKEN=…` then `echo $TOKEN` returns empty. Agents work
around it by re-stating context in every command. Stateful bash removes that friction
and lets the model treat bash as a real REPL.

## What

Foreground bash becomes **stateful per session**: shell state is captured after each
call and restored before the next. Behavior is **on by default** — there is no
feature flag and no opt-in.

bashkit 0.7.2 provides the primitives (see the `Bash` struct, `Snapshot`,
`SnapshotOptions`, `ShellState`):

- `snapshot_with_options(SnapshotOptions { exclude_filesystem: true, .. })` — capture
  shell state only. The VFS is **excluded** because it is external and already
  durable; including it would double-store and fight the live adapter.
- `restore_snapshot(&mut self, bytes)` — overlay persisted shell state onto an
  existing, already-configured `Bash`.
- The keyed variants (`snapshot_to_bytes_keyed` / `restore_snapshot_keyed`,
  HMAC-SHA256) are used so the blob is tamper-checked across the storage boundary.

### Per-call flow (foreground `execute_with_context`)

1. Load the session's shell-state blob from `SessionStorageStore` (the `storage_store`
   already on `ToolContext`) under a reserved key (e.g. `__bash_shell_state__`),
   base64-decoded (KV values are `String`).
2. Build the `Bash` exactly as today (fs adapter, env, limits, observability hooks,
   locale). This must happen via the builder so the custom fs/hooks stay attached.
3. `restore_snapshot_keyed(bytes, key)` to overlay the persisted shell state onto that
   configured instance. **Do not** use `from_snapshot` — it builds a bare `Bash` and
   would drop the fs adapter and hooks.
4. Resolve cwd: if the `working_dir` argument is supplied, honor it (and it becomes the
   new persisted cwd); if omitted, use the restored cwd. This is a behavior change —
   `working_dir` no longer defaults to `/workspace` once a session has state.
5. `exec_streaming` as today.
6. `snapshot_with_options({ exclude_filesystem: true })`, HMAC-key, base64, and write
   back to storage. Session counters carry forward via the snapshot fields so
   `SessionLimits` stay cumulative across calls.

### Background bash stays stateless

`execute_background` does **not** load or persist shell state. It runs with the
default fresh environment as it does today. Rationale: background runs can overlap a
foreground call, and with no worker affinity (below) coordinating writes would require
distributed concurrency control for marginal benefit. File side effects still land
through the shared session filesystem; only shell mutations are scoped to that run.

## Concurrency

Workers are stateless with no session affinity — *"activities within a turn can land
on different workers"* (`specs/dismissed-options.md`). This rules out an in-process
`Arc<Mutex<Bash>>` REPL cache: a follow-up call may execute on a different worker.
State **must** be externalized, which the snapshot-to-storage design does.

- **Cross-session:** fully isolated — distinct `session_id`, distinct storage key,
  distinct `Bash`. Runs in parallel; nothing to coordinate.
- **Within a session:** the agent loop is sequential (one foreground tool call per
  turn, turns processed one at a time), so foreground calls do not race. Last-writer-
  wins on the blob is acceptable. Background bash never writes shell state, so it
  cannot clobber the foreground REPL. No lock or CAS is required for the initial
  design; if truly concurrent foreground writes ever appear, add an optimistic `seq`
  + compare-and-swap on the blob.

## Edge cases

- **Version mismatch / corruption:** `restore_snapshot*` returns `Result`. On the
  bashkit `Snapshot.version` bumping (library upgrade) or a decode/HMAC failure, log
  and start from fresh state — never hard-fail the call.
- **Secrets in state:** `export SECRET=…` is captured into the blob. Use the keyed
  (HMAC) snapshot and store via the encrypted secret path rather than plain KV; the
  blob never leaves the trust boundary unsigned. See TM-BASH-017.
- **Size growth:** large function bodies / env can bloat the blob. Cap blob size; on
  overflow, reset shell state with a warning rather than refusing the call.
- **Reset escape hatch:** a way to clear a session's shell state (delete the key),
  returning the next call to a clean environment.

## Implementation

`crates/core/src/capabilities/virtual_bash.rs` — `execute_with_context` gains the
load/restore (before build/exec) and snapshot/persist (after exec) steps; a small
helper encapsulates storage key, base64, HMAC keying, and graceful-reset fallback.
`execute_background` is unchanged. bashkit version is 0.7.2 (`Cargo.lock`).
