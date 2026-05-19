# Everruns Coding CLI

A minimal terminal coding agent built on `everruns-runtime` (in-process, no
server). Think codex/claude-code in spirit, embedded as a single binary that
talks to your codebase.

The example lives at the workspace edge — it's a standalone Cargo project that
depends on the public runtime crate the same way an external embedder would.

## What it does

- **TUI** (ratatui) chat: scrolling transcript, single-line input, status bar,
  modal approval bar (when `--ask` is on)
- **Real-filesystem tools** via the built-in `session_file_system` capability
  on top of `RealDiskFileStore`: `read_file`, `write_file`, `edit_file`,
  `list_directory`, `grep_files`, `delete_file`, `stat_file`.
- **Plus a custom `bash`** tool that runs `bash -lc` from the workspace root.
- **Curated built-in capabilities** wired beyond filesystem:
  - `agent_instructions` — re-reads `AGENTS.md` every turn (live reload).
  - `skills` — discovers `SKILL.md` files under `/.agents/skills/{name}/`;
    exposes `list_skills` / `activate_skill`.
  - `infinity_context` — keeps long sessions usable by trimming older history
    out of the live prompt while keeping it queryable via `query_history`.
  - `stateless_todo_list` — `write_todos` for multi-step task tracking.
  - `loop_detection` — safety net against the model retrying the same
    failing tool call in a loop.
  - `prompt_caching` — Anthropic prompt-caching markers; free token savings
    on long system prompts (including AGENTS.md).
  - `duckduckgo` — `duckduckgo_search` tool. Free, no API key. Hits the
    DuckDuckGo Instant Answer API for definitions, abstracts, and related
    topics. Useful for the agent to look up docs/concepts without leaving
    the session.
  - `web_fetch` — HTTP GET/HEAD with optional markdown/text conversion.
    Saved responses land on disk via the same `RealDiskFileStore` stack,
    so the blocklist and approval gate apply.

Note on parallel tool calls: independent tool calls already execute in
parallel when the LLM provider batches them in one assistant turn (Anthropic
and OpenAI both do). No capability needed.
- **Approval prompts (opt-in via `--ask`)**: off by default — the agent acts
  autonomously, exactly like codex/claude-code with `--auto`. Pass `--ask` to
  prompt `y/n` before every write/edit/delete on disk and every `bash`
  command. Writes show a unified diff in the approval body. `--print` mode
  always auto-approves regardless of `--ask`.
- **Write blocklist**: writes into `.git/`, `node_modules/`, `target/`,
  `dist/`, `build/`, `.next/`, `.venv/`, `venv/`, `.tox/`, `.gradle/` are
  rejected at any depth. Read access is unrestricted.
- **Tool-result visibility**: the transcript shows a per-tool summary
  (e.g. `read_file ✓  /workspace/crates/runtime/src/runtime.rs (45/788 lines)`,
  `` bash ✓  `cargo test` exit=0 ``, `write_todos ✓`, `list_skills ✓`).
- **Provider selection** via env vars: `OPENAI_API_KEY` → OpenAI (`gpt-5.5`),
  `ANTHROPIC_API_KEY` → Anthropic (`claude-sonnet-4-5`), otherwise falls back
  to `llmsim` (offline). OpenAI is preferred when both keys are present so the
  default model stays `gpt-5.5`.
- **Slash commands** (TUI): `/help`, `/tools`, `/cwd`, `/model <provider>/<id>`, `/clear`, `/quit`.
  Typing `/` opens suggestions; Tab accepts the first suggestion. `/model`
  with no argument shows the current model and suggested model IDs, while
  `/model openai/gpt-5.5`, `/model anthropic/claude-sonnet-4-5`, or
  `/model llmsim/llmsim-coding-cli` changes the active provider/model for
  subsequent turns. Bare model IDs still target the current provider.
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
ercode --provider llmsim -p "hi"
```

## Flags

| Flag                       | Description                                                          |
| -------------------------- | -------------------------------------------------------------------- |
| `-C, --cwd <PATH>`         | Workspace root (default: current dir)                                |
| `--provider <P>`           | Force `anthropic`, `openai`, or `llmsim` (default: env-detected)     |
| `-m, --model <ID>`         | Override the model id for the chosen provider                        |
| `-p, --print <PROMPT>`     | Run one prompt non-interactively and print the result                |
| `--ask`                    | Prompt before every destructive tool call (off by default)           |

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
