# ercode — a complete coding agent on the public `everruns` API

`ercode` is a small terminal coding agent built only on the public
[`everruns`](../../crates/everruns) crate. It demonstrates provider-owned Git
workspace heads, typed session resume, `Environment` binding, `WorkspacePolicy`,
and Framework coding capabilities without importing `everruns-core`,
`everruns-host`, or `everruns-local` directly.

The example retains the historical coding-agent experience while exercising
the Framework facade: a multiline ratatui composer, model/tool status,
optional destructive-tool approval, open provider and model selection, MCP
servers, durable conversation resume, and model switching without losing
history. Its profile enables `session_file_system`, `bashkit_shell`, live
`agent_instructions`, `skills`, `infinity_context`, `stateless_todo_list`,
`loop_detection`, `prompt_caching`, `tool_output_persistence`, `web_fetch`, and
`duckduckgo`.

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

Provider auto-detection checks OpenAI, Anthropic, OpenRouter, then Ollama. You
can also select one explicitly and pass its provider-visible model id:

```bash
export OPENAI_API_KEY=sk-...
cargo run -p everruns-coding-cli -- --provider openai --model gpt-5.5 \
  --head docs --base main "improve the README"
```

The supported service configurations are `openai`, `anthropic`, `openrouter`,
`ollama`, and deterministic `llmsim`; each uses its public driver crate.
`--reasoning-effort <LEVEL>` attaches a portable per-turn control.

Interactive mode supports `/help`, `/tools`, `/cwd`, `/mcp`, `/clear`,
`/model`, `/model <provider>/<model>`, and `/quit`. Model switching builds a new
immutable Agent snapshot and resumes the same typed Session through its durable
event history and exact workspace-head binding. Bare model ids retain the
current provider. Pass `--ask` to confirm filesystem writes and shell tools.
One-shot `--print` never prompts. Workspace `.mcp.json` files are discovered
automatically, or `--mcp-config <FILE>` selects one explicitly.

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
the application—not Framework—must coordinate concurrent edits.

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
errors, explicit sharing, no implicit deletion, historical flags, open provider
selection, MCP parsing, and the complete coding capability catalog. Source and
manifest guards reject internal Everruns dependencies, process-global workspace
state, direct `tokio::fs`, and duplicate filesystem tools. Changes under this
example are in the Rust CI path filter, so workspace Clippy/tests compile the
binary and all example tests on pull requests.
