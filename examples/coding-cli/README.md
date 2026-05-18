# Everruns Coding CLI

A minimal terminal coding agent built on `everruns-runtime` (in-process, no
server). Think codex/claude-code in spirit, embedded as a single binary that
talks to your codebase.

The example lives at the workspace edge — it's a standalone Cargo project that
depends on the public runtime crate the same way an external embedder would.

## What it does

- **TUI** (ratatui) chat: scrolling transcript, single-line input, status bar,
  modal approval bar for destructive tools
- **Real-filesystem tools**: `read_file`, `write_file`, `edit_file`,
  `list_directory`, `grep`, `bash` — all rooted at a workspace path, with
  traversal protection
- **Human-in-the-loop approvals**: `write_file`, `edit_file`, and `bash`
  request explicit approval (`y` / `n`) before running. `edit_file` shows a
  unified diff in the prompt. `--yes` auto-approves; `--print` mode implies
  auto-approve.
- **Write blocklist**: writes into `.git/`, `node_modules/`, `target/`,
  `dist/`, `build/`, `.next/`, `.venv/`, `venv/`, `.tox/`, `.gradle/` are
  rejected at any depth. Read access is unrestricted.
- **Tool-result visibility**: the transcript shows a per-tool summary
  (e.g. `read_file ✓  crates/runtime/src/runtime.rs (45/788 lines)`,
  `` bash ✓  `cargo test` exit=0 ``) and, for `edit_file`, the unified diff
  inline.
- **AGENTS.md / CLAUDE.md / .agents.md** loaded from the workspace root and
  injected into the system prompt
- **Provider selection** via env vars: `OPENAI_API_KEY` → OpenAI (`gpt-5.5`),
  `ANTHROPIC_API_KEY` → Anthropic (`claude-sonnet-4-5`), otherwise falls back
  to `llmsim` (offline). OpenAI is preferred when both keys are present so the
  default model stays `gpt-5.5`.
- **Slash commands**: `/help`, `/tools`, `/cwd`, `/model`, `/clear`, `/quit`
- **`--print`** one-shot mode for CI smoke tests

## Install

```bash
cargo install --path examples/coding-cli --locked
```

This drops the `ercode` binary into `~/.cargo/bin/`.

## Run

Interactive TUI in the current repo:

```bash
ercode
# or, without installing:
cargo run -p everruns-coding-cli
```

Against a different workspace:

```bash
ercode -C /path/to/repo
```

One-shot prompt (no TUI):

```bash
ercode --provider anthropic -p "List the top-level crates and summarize each in one line."
```

With Doppler secrets:

```bash
doppler run -- ercode -p "Show me the runtime spec."
```

Offline (no API key required):

```bash
ercode --provider sim -p "hi"
```

## Flags

| Flag                       | Description                                                          |
| -------------------------- | -------------------------------------------------------------------- |
| `-C, --cwd <PATH>`         | Workspace root (default: current dir)                                |
| `--provider <P>`           | Force `anthropic`, `openai`, or `sim` (default: env-detected)        |
| `-m, --model <ID>`         | Override the model id for the chosen provider                        |
| `-p, --print <PROMPT>`     | Run one prompt non-interactively and print the result                |
| `--yes`                    | Auto-approve every destructive tool call (write/edit/bash)           |

`RUST_LOG` is honored for the underlying tracing layer (writes to stderr).

## How it's wired

- `src/runtime.rs` — plugs `RealDiskFileStore` (from `everruns-runtime`)
  rooted at the workspace, wrapped by two policy decorators
  (`WriteBlocklistFileStore` and `ApprovalGatingFileStore`), into
  `RuntimeBackends.file_store`. Registers the built-in
  `AgentInstructionsCapability` (live-reloads AGENTS.md every turn),
  `FileSystemCapability` (read/write/edit/list/grep/delete/stat tools on real
  disk via the FileStore stack), and a tiny custom `CodingBashCapability` for
  the shell tool. Picks a driver (Anthropic / OpenAI / llmsim).
- `src/file_store_decorators.rs` — `WriteBlocklistFileStore` and
  `ApprovalGatingFileStore`. Both implement `SessionFileStore +
  RuntimeFileStore` and compose freely. EVE-478 plans to ship these in
  `everruns-runtime`; until then they live here.
- `src/tools.rs` — `BashTool` only. Built-in `virtual_bash` runs against the
  VFS, not the real workspace, so the shell tool stays custom.
- `src/approval.rs` — `ApprovalGate` and the request enum; the gate is shared
  between the bash tool and the FileStore decorator.
- `src/app.rs` + `src/main.rs` — ratatui TUI and one-shot CLI driver.

## Caveats

- Single-turn rendering: assistant messages appear after the turn completes
  rather than streaming token-by-token (the runtime emits delta events; wiring
  them to the UI is a follow-up).
- No conversation persistence: the in-memory runtime drops history when the
  binary exits.
- Bash tool has a 120s timeout and a 64KiB stdout cap. Long-running jobs aren't
  yet supported as background tools.
- The bash approval prompt shows the command string only — sub-commands
  spawned by it are not pre-listed.
- Write blocklist matches directory names case-sensitively at any depth; it is
  intentionally conservative, not exhaustive.
- `FileStore` decorators live in this example. EVE-478 will move
  `ApprovalGatingFileStore` and `WriteBlocklistFileStore` (or equivalents)
  into `everruns-runtime` so other embedders can compose them.
