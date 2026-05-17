# Everruns Coding CLI

A minimal terminal coding agent built on `everruns-runtime` (in-process, no
server). Think codex/claude-code in spirit, embedded as a single binary that
talks to your codebase.

The example lives at the workspace edge — it's a standalone Cargo project that
depends on the public runtime crate the same way an external embedder would.

## What it does

- **TUI** (ratatui) chat: scrolling transcript, single-line input, status bar
- **Real-filesystem tools**: `read_file`, `write_file`, `edit_file`,
  `list_directory`, `grep`, `bash` — all rooted at a workspace path, with
  traversal protection
- **AGENTS.md / CLAUDE.md / .agents.md** loaded from the workspace root and
  injected into the system prompt
- **Provider selection** via env vars: `ANTHROPIC_API_KEY` → Anthropic,
  `OPENAI_API_KEY` → OpenAI, otherwise falls back to `llmsim` (offline)
- **Slash commands**: `/help`, `/tools`, `/cwd`, `/model`, `/clear`, `/quit`
- **`--print`** one-shot mode for CI smoke tests

## Run

Interactive TUI in the current repo:

```bash
cargo run --manifest-path examples/coding-cli/Cargo.toml
```

Against a different workspace:

```bash
cargo run --manifest-path examples/coding-cli/Cargo.toml -- -C /path/to/repo
```

One-shot prompt (no TUI):

```bash
cargo run --manifest-path examples/coding-cli/Cargo.toml -- \
  --provider anthropic -p "List the top-level crates and summarize each in one line."
```

With Doppler secrets:

```bash
doppler run -- cargo run --manifest-path examples/coding-cli/Cargo.toml -- -p "Show me the runtime spec."
```

Offline (no API key required):

```bash
cargo run --manifest-path examples/coding-cli/Cargo.toml -- --provider sim -p "hi"
```

## Flags

| Flag                       | Description                                                          |
| -------------------------- | -------------------------------------------------------------------- |
| `-C, --cwd <PATH>`         | Workspace root (default: current dir)                                |
| `--provider <P>`           | Force `anthropic`, `openai`, or `sim` (default: env-detected)        |
| `-m, --model <ID>`         | Override the model id for the chosen provider                        |
| `-p, --print <PROMPT>`     | Run one prompt non-interactively and print the result                |

`RUST_LOG` is honored for the underlying tracing layer (writes to stderr).

## How it's wired

- `src/tools.rs` — six tool impls of the `everruns_core::tools::Tool` trait,
  each rooted at a `Workspace` that rejects traversal outside the root.
- `src/runtime.rs` — wraps the tools in a custom `Capability`, picks a driver
  (Anthropic / OpenAI / llmsim), and seeds a single
  harness/agent/session into an `InProcessRuntime`.
- `src/instructions.rs` — loads `AGENTS.md` / `CLAUDE.md` / `.agents.md` from
  the workspace root and folds them into the harness system prompt.
- `src/app.rs` + `src/main.rs` — ratatui TUI and one-shot CLI driver.

## Caveats

- Single-turn rendering: assistant messages appear after the turn completes
  rather than streaming token-by-token (the runtime emits delta events; wiring
  them to the UI is a follow-up).
- No conversation persistence: the in-memory runtime drops history when the
  binary exits.
- Bash tool has a 120s timeout and a 64KiB stdout cap. Long-running jobs aren't
  yet supported as background tools.
