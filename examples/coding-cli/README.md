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
- **Curated built-in and CLI-owned capabilities** wired beyond filesystem:
  - `coding_cli_environment_context` — CLI-owned context injection for the
    current workspace root, shell, local date/timezone, and Git identity/branch.
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
  - `tool_output_persistence` — bash output is summarized inline and the
    full stdout/stderr streams are saved under `/outputs/` inside the current
    session folder so the agent can inspect them with `read_file`.

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
  The `bash` tool also accepts an `output` verbosity argument matching the
  built-in sandbox exec tools.
- **Provider selection** via env vars: `OPENAI_API_KEY` → OpenAI (`gpt-5.5`),
  `ANTHROPIC_API_KEY` → Anthropic (`claude-sonnet-4-5`), `OPENROUTER_API_KEY`
  → OpenRouter (`openai/gpt-5.2`), `OLLAMA_BASE_URL` or `OLLAMA_API_KEY` →
  Ollama (`llama3.2`), otherwise falls back to `llmsim` (offline). OpenAI is
  preferred when multiple provider env vars are present so the default model
  stays `gpt-5.5`.
- **MCP servers** via a workspace `.mcp.json` (remote HTTP). Tools from each
  configured server are discovered and become available to the agent with
  `mcp_<server>__<tool>` names. Example `.mcp.json` at the workspace root:

  ```json
  {
    "mcpServers": {
      "docs": {
        "type": "http",
        "url": "https://example.com/mcp",
        "headers": { "Authorization": "Bearer <token>" }
      }
    }
  }
  ```

  See `specs/runtime-mcp.md`. The CLI builds with the runtime's `mcp-stdio`
  feature so local-process MCP servers can be added as that path lands.
- **Slash commands** (TUI): `/help`, `/tools`, `/cwd`, `/mcp`, `/model <provider>/<id>`, `/clear`, `/quit`.
  Typing `/` opens suggestions; Tab accepts the first suggestion. `/model`
  with no argument shows the current model and suggested model IDs, while
  `/model openai/gpt-5.5`, `/model anthropic/claude-sonnet-4-5`,
  `/model openrouter/openai/gpt-5.2`, `/model ollama/llama3.2`, or
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

OpenRouter, using its OpenAI-compatible Responses endpoint:

```bash
OPENROUTER_API_KEY=sk-or-... ercode --provider openrouter -m openai/gpt-5.2 -p "hi"
```

Local Ollama, using its OpenAI-compatible Responses endpoint:

```bash
OLLAMA_BASE_URL=http://localhost:11434/v1 ercode --provider ollama -m llama3.2 -p "hi"
```

## Flags

| Flag                       | Description                                                          |
| -------------------------- | -------------------------------------------------------------------- |
| `-C, --cwd <PATH>`         | Workspace root (default: current dir)                                |
| `--provider <P>`           | Force `anthropic`, `openai`, `openrouter`, `ollama`, or `llmsim`     |
| `-m, --model <ID>`         | Override the model id for the chosen provider                        |
| `-p, --print <PROMPT>`     | Run one prompt non-interactively and print the result                |
| `--ask`                    | Prompt before every destructive tool call (off by default)           |
| `--session <ID>`           | Resume a previous session by id (its JSONL log is replayed)          |
| `--session-dir <PATH>`     | Override the parent directory for session folders (default: XDG data dir) |

`RUST_LOG` is honored for the underlying tracing layer (writes to stderr).

Provider env vars:

| Env var                         | Description                                                 |
| ------------------------------- | ----------------------------------------------------------- |
| `OPENAI_API_KEY`                | Select OpenAI unless `--provider` overrides it              |
| `ANTHROPIC_API_KEY`             | Select Anthropic when OpenAI is not configured              |
| `OPENROUTER_API_KEY`            | Select OpenRouter when OpenAI/Anthropic are not configured  |
| `OPENROUTER_BASE_URL`           | Optional, defaults to `https://openrouter.ai/api/v1`        |
| `OLLAMA_BASE_URL`               | Select Ollama, defaults to `http://localhost:11434/v1`      |
| `OLLAMA_API_KEY`                | Optional, defaults to `ollama` for local Ollama             |
| `EVERRUNS_CLI_MODEL`            | Override the auto-selected default model                    |
| `EVERRUNS_CLI_REASONING_EFFORT` | OpenAI-only reasoning effort override                       |

## Session persistence

Every run writes durable local artifacts into a per-session folder under
the platform-native user data directory:

| OS      | Default location                                                 |
|---------|------------------------------------------------------------------|
| Linux   | `$XDG_DATA_HOME/ercode/sessions/<session_id>/` (typically `~/.local/share/…`) |
| macOS   | `~/Library/Application Support/ercode/sessions/<session_id>/` |
| Windows | `%APPDATA%\ercode\sessions\<session_id>\`                   |

The event log lives at `<session_folder>/events.jsonl`. Tool output persisted
by `tool_output_persistence` lives under `<session_folder>/outputs/`. On first
resume after upgrading from the old flat-log layout, ercode copies
`<sessions_dir>/<session_id>.jsonl` into the session folder if no
`events.jsonl` exists yet. The folder layout leaves room for other
session-scoped stores, such as key/value data, to sit beside the event log.

One serialized `Event` per line, flushed after every write. On Unix
`events.jsonl` is created with `0o600` and its parent session folder
is set to `0o700` (both owner-only) because session logs contain user
prompts, tool arguments, tool output, and the reasoning artifacts
discussed below. The session id is generated fresh on every plain
`ercode` invocation and printed in the startup banner (`[session]
session_… (folder: …; log: …)`).

The event types kept on disk are those that round-trip into the
conversation (`input.message`, `output.message.completed`,
`tool.completed`) plus the agent reasoning artifacts ercode needs to
restore the live transcript view and provider continuation state on
resume (`reason.completed` carries the safe `text_preview` narration;
`reason.item` carries opaque/encrypted reasoning context curated by the
provider, such as OpenAI Responses reasoning items). Assistant
`thinking` / `thinking_signature` are persisted alongside
`output.message.completed` — providers that resume via encrypted
reasoning continuation (e.g. OpenAI Responses replays
`thinking_signature` as `encrypted_content`) cannot continue without
them. Streaming `*.delta` events and lifecycle markers
(`reason.started`, `reason.thinking.*`, `output.message.started`) are
dropped from the log — they are live status signals only and the delta
types would inflate the file O(n²) without adding resume value.

This persistence contract is **local-store**, not user-facing
transcript export. On Unix, the per-session folder is set to `0o700`
and the `events.jsonl` file inside it to `0o600` on every open, both
under the platform-native user data directory; treat the folder
contents as sensitive (see [Sensitivity](#sensitivity) below).

To continue a previous conversation, pass `--session <id>`:

```bash
ercode --session session_019e3db018a17450aba5407af5777237
```

On resume the log is replayed, messages are reconstructed from the
recorded events and seeded into the in-memory message store, then the
agent picks up where it left off. Events tagged with a different
`session_id` (e.g., from a tampered or copied log) are skipped with a
warning. The same JSONL file is then re-opened in append mode for the
new run; `Event.sequence` continues monotonically past the highest
replayed value.

`--session-dir <PATH>` overrides the parent storage location (useful for
keeping per-workspace session histories in
`<workspace>/.ercode/sessions/`).

### Sensitivity

**Treat session logs as you would shell history.** Each line is the
serialized `Event` that fired during a turn, which may include:

- Every prompt you typed.
- Tool call arguments — including paths and any string the agent passed
  to `bash`, `write_file`, `edit_file`, `web_fetch`, etc.
- Tool output — `bash` stdout/stderr, file contents, HTTP response
  bodies (capped per-tool but not redacted).
- Agent reasoning artifacts — `reason.completed.text_preview`
  narration, `reason.item` opaque/encrypted reasoning context, and the
  `thinking` / `thinking_signature` fields on assistant messages.
  Persisting these is what lets `--session <id>` resume restore the
  transcript view and lets providers (e.g. OpenAI Responses) continue
  encrypted reasoning across resumes; they are deliberately not
  redacted from the local log.

There is no retention policy or rotation — files grow until you delete
them. If you'd rather a session not be persisted, point `--session-dir`
at a path you can wipe (e.g., a `tmpfs`) or delete the JSONL after the
run.

## How it's wired

- `src/runtime.rs` — registers a platform `SessionFileSystemFactory` that
  routes normal paths through a `RealDiskFileStore` rooted at the workspace
  and routes `/outputs/` through the current session folder, then wraps it
  with two policy decorators
  (`WriteBlocklistFileStore` and `ApprovalGatingFileStore`, both also from
  `everruns-runtime`). Registers the built-in
  `AgentInstructionsCapability` (live-reloads AGENTS.md every turn),
  `FileSystemCapability` (read/write/edit/list/grep/delete/stat tools on real
  disk via the platform session filesystem stack),
  `ToolOutputPersistenceCapability` (saves large exec output), and a tiny
  custom `CodingBashCapability` for the shell tool. Picks a driver
  (Anthropic / OpenAI / llmsim).
- `src/tools.rs` — `BashTool` only. Built-in `virtual_bash` runs against the
  VFS, not the real workspace, so the shell tool stays custom.
- `src/approval.rs` — `ApprovalGate` and the request enum; implements
  `everruns_runtime::FileApprovalGate` so it can be plugged directly into the
  approval decorator. The gate is shared between the bash tool and the
  session filesystem approval decorator.
- `src/app.rs` + `src/main.rs` — ratatui TUI and one-shot CLI driver.

## Caveats

- Single-turn rendering: assistant messages appear after the turn completes
  rather than streaming token-by-token (the runtime emits delta events; wiring
  them to the UI is a follow-up).
- Persistence is event-log only: messages are reconstructed from events on
  resume. There's no separate snapshot of agent state (skills cache, todos,
  budget counters); each new run rebuilds in-memory state from scratch.
- Bash tool has a 120s timeout and a 1MiB-per-stream capture cap. Long-running
  jobs aren't yet supported as background tools.
- The bash approval prompt shows the command string only — sub-commands
  spawned by it are not pre-listed.
- Write blocklist matches directory names case-sensitively at any depth; it is
  intentionally conservative, not exhaustive.
