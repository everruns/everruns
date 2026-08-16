# ercode, a small coding agent on the public `everruns` API

`ercode` is a small terminal coding agent built only on the public
[`everruns`](../../crates/everruns) crate. Agent creation is the center of the
example: it selects a model, adds typed coding capabilities, and binds a safe
workspace policy. The surrounding CLI demonstrates provider-owned Git workspace
heads, typed session resume, and direct session/workspace binding without importing
`everruns-core`, `everruns-host`, or `everruns-local` directly.

The deliberately small profile uses typed values for `session_file_system`,
`bashkit_shell`, live `agent_instructions`, `skills`, `stateless_todo_list`,
`web_fetch`, and `duckduckgo`. The CLI keeps a multiline composer, optional
destructive-tool approval, and durable conversation resume. It
supports OpenAI and a deterministic offline simulator; multi-provider routing
belongs in an application, not this example.

The important safety model is simple:

- a new invocation gets a new **isolated** Git head by default;
- the selected head is bound permanently before the session runs;
- filesystem tools operate under `/workspace` on that exact head;
- the default coding policy permits ordinary reads and writes but continues to
  protect hidden, sensitive, dependency, and build paths;
- `--read-only` denies all writes;
- dropping the process never destroys a worktree or branch.

The CLI prints its typed session id, workspace/head ids, access mode, base, and
state directory before the first prompt. Keep the session id when you want to
resume or explicitly share that recorded head.

## Run offline or with a live provider

Offline mode is deterministic and needs no credential or network:

```bash
cargo run -p everruns-coding-cli -- --offline --head review --base main \
  "inspect the repository"
```

With `OPENAI_API_KEY` set, the CLI uses OpenAI and accepts a model id:

```bash
export OPENAI_API_KEY=sk-...
cargo run -p everruns-coding-cli -- --model gpt-5.6-terra \
  --head docs --base main "improve the README"
```

Without that variable, the CLI falls back to the deterministic simulator.
`--reasoning-effort <LEVEL>` attaches a portable per-turn control.

Interactive mode supports `/help`, `/tools`, `/cwd`, `/clear`,
`/model`, and `/quit`. Pass `--ask` to confirm filesystem writes and shell
tools. One-shot `--print` never prompts.

`-C/--cwd <REPOSITORY>` selects a trusted local Git repository when creating a
new head. Resume and shared-head modes reopen the workspace recorded in durable
Framework state and therefore do not accept `--cwd`. Framework state defaults
to the operating system's user state directory; `--state-dir <DIR>` selects an
explicit persistent location. With no prompt, `ercode` starts its inline TUI
(`Esc` exits). Enter sends and Alt/Shift-Enter inserts a newline. Every turn
uses the same session and head.

## Two isolated heads from the same base

Run these in separate terminals. The visible names are descriptive; the local
provider creates distinct durable branches and worktrees, so writes do not
collide and the original checkout is not edited.

```bash
cargo run -p everruns-coding-cli -- --offline --head left --base main
cargo run -p everruns-coding-cli -- --offline --head right --base main
```

Reusing the same `--head` name creates another isolated head. Identity is the
printed `head` id, not the display name.

## Resume the exact recorded head

Copy the printed typed session id from an earlier run:

```bash
cargo run -p everruns-coding-cli -- --offline \
  --resume session_0123456789abcdef0123456789abcdef
```

Resume continues that session's history and reopens its exact recorded head.
It never substitutes the repository checkout or a newly created head. If the
recorded head is archived, destroyed, or otherwise unavailable, resume fails
clearly.

## Explicit shared-head behavior

Sharing has two explicit steps. First create a head as shared and keep the
printed session id:

```bash
cargo run -p everruns-coding-cli -- --offline --head team --shared
```

Then start a distinct session on the head recorded by that session id:

```bash
cargo run -p everruns-coding-cli -- --offline \
  --shared-head session_0123456789abcdef0123456789abcdef
```

`--shared-head` rejects a session recorded on an isolated head; use `--resume`
for that case. Shared sessions intentionally address the same mutable files, so
the application, not Framework, must coordinate concurrent edits.

## Lifecycle and cleanup

`ercode` never removes a worktree or branch on normal exit, handle drop, failed
turn, or resume. Inspect provider-created worktrees with `git worktree list`.
The public lifecycle operation `WorkspaceHead::destroy()` is the explicit way
for an embedding application to remove provider-owned worktree storage; the
local Git provider retains its branch for recovery. Archive and destroy are not
implicit CLI exit behavior, and deleting the state directory is not a safe
substitute for lifecycle APIs.

## Test and CI contract

```bash
cargo test -p everruns-coding-cli
cargo run -p everruns-coding-cli -- --help
```

The package tests create temporary Git repositories and prove isolated writes,
selected-head paths, policy enforcement, typed reopen/resume, missing-head
errors, explicit sharing, no implicit deletion, and owner-defined typed
capability values. Source and
manifest guards reject internal Everruns dependencies, process-global workspace
state, direct `tokio::fs`, and duplicate filesystem tools. Changes under this
example are in the Rust CI path filter, so workspace Clippy/tests compile the
binary and all example tests on pull requests.
