# CLI Specification

## Overview

`everruns` — command-line interface for the Everruns platform. Manages agents, sessions, chat, and file sync.

**Crate:** `crates/cli/`

**Global Flags:**
- `-o, --output` — Output format: `text` (default), `json`, `yaml`
- `-q, --quiet` — Suppress non-essential output
- `--profile <name>` — Credential profile (default: `default`)

**Configuration (precedence order):**
1. CLI flags (`--api-key`, `--api-url`)
2. Environment variables (`EVERRUNS_API_KEY`, `EVERRUNS_API_URL`)
3. Credential file (`<config_dir>/everruns/credentials.json` — Linux: `~/.config/everruns/`, macOS: `~/Library/Application Support/everruns/`)

**Credential File:**
- Multi-profile support: `{ "profiles": { "default": { "api_url", "api_key", "org_id" } }, "current_profile": "default" }`
- File permissions: `0600` on Unix
- Managed by `everruns login` / `everruns logout`

## Commands

### `everruns login`

Interactive authentication. Uses localhost HTTP callback OAuth flow.

- `login` — Open browser for OAuth login, receive a personal access token via localhost callback
- `login --token` — Paste a personal access token directly (headless/SSH fallback)

Flow: CLI → `POST /v1/auth/cli/start` → open browser → user logs in → server redirects to `localhost:{port}/callback?code=...` → CLI calls `POST /v1/auth/cli/exchange` → receives a personal access token + user info + orgs → interactive org selection → stores in credential file.

### `everruns logout`

Remove stored credentials for current profile.

### `everruns status`

Show current user, API URL, org, and masked personal access token.

### `everruns orgs`

Organization management.

- `orgs` — List organizations (marks current with `*`)
- `orgs select` — Interactive org picker

### `everruns agents`

Agent CRUD. Create from TOML/YAML/JSON/Markdown files or CLI flags.

- `create --file <path> [--initial-files-dir <dir>] [--writable]` — send file to server import API; the CLI normalizes TOML to JSON and otherwise forwards the file as-is. Upserts when `id:` is present in the definition. If `--file` is omitted and `./agent.toml` exists, the CLI uses it automatically unless inline creation flags are present. `--initial-files-dir` recursively collects non-hidden text files from the directory and injects them as read-only `initial_files`. `--writable` makes collected files writable. Alternatively, `initial_files` can be specified directly in the definition as a list of relative paths — each entry is resolved relative to the agent file's parent directory, directories are walked recursively, and files are collected with the same security rules as `--initial-files-dir`.
- `create --name <n> --system-prompt <s> [--description <d>] [--model <m>] [--tag <t>]` — create from CLI flags
- `update --file <path> [--initial-files-dir <dir>] [--writable]` — send file to server import API; TOML is normalized client-side, and `./agent.toml` is used automatically when `--file` is omitted, no inline update flags are present, and no positional `<id>` is provided. Requires `id:` in the definition for upsert. `--initial-files-dir` and definition `initial_files` work the same as in create.
- `update <id> --name <n> --system-prompt <s> [--description <d>] [--model <m>] [--tag <t>]` — update from CLI flags
- `list`
- `get <id>`
- `delete <id>` (soft archive)
- `search <query>` — full-text search over agents
- `stats <id>` — usage statistics (session/execution counts, token totals)
- `copy <id>` — duplicate an agent into a new agent
- `export <id> [--output <file>]` — export the agent definition (defaults to stdout)
- `import-example <name>` — create an agent from a built-in example

Version management under `agents versions`:

- `versions list <id>`
- `versions create <id> [--summary <s>] [--change-kind <manual|patch|minor|major|import|rollback|fork>]` — snapshot the current config
- `versions set-default <id> <version_id>` — make a version the active default
- `versions diff <id> <from> <to>` — diff two versions
- `versions fork <id> <version_id> --name <n> [--display-name <d>] [--description <d>]` — fork a version into a new agent
- `versions rollback <id> <version_id> [--save-version] [--summary <s>]` — roll back to a version

#### Initial Files Hidden Path Policy

`initial_files` collection (whether from `--initial-files-dir` or definition globs) gates hidden (dot-prefixed) path components to prevent accidental upload of host secrets. The policy has three layers, evaluated per path component:

1. **Hard-deny floor** — known credential / version-control / shell-history paths are never uploaded. Examples: `.env`, `.env.local`, `.envrc`, `.ssh`, `.gnupg`, `.aws`, `.azure`, `.gcloud`, `.kube`, `.docker`, `.npmrc`, `.yarnrc`, `.pypirc`, `.netrc`, `.cargo`, `.git`, `.hg`, `.svn`, `.bash_history`, `.zsh_history`, `.python_history`, `.node_repl_history`. The full list lives in `crates/cli/src/commands/agents.rs::DENIED_DOT_ENTRIES`. The hard-deny floor cannot be bypassed by user opt-in.
2. **Built-in allowlist** — common dev-ecosystem assets are allowed by default. Exact basenames: `.agents`, `.github`, `.vscode`, `.claude`, `.cursor`, `.mcp.json`, `.gitignore`, `.gitattributes`, `.editorconfig`, `.prettierrc`, `.prettierrc.json`, `.prettierrc.yaml`, `.prettierrc.yml`, `.prettierrc.js`, `.prettierrc.cjs`, `.prettierrc.mjs`, `.eslintrc`, `.eslintrc.json`, `.eslintrc.yaml`, `.eslintrc.yml`, `.eslintrc.js`, `.eslintrc.cjs`, `.eslintignore`, `.nvmrc`, `.node-version`, `.python-version`, `.tool-versions`, `.dockerignore`, `.rubocop.yml`. The authoritative list lives in `crates/cli/src/commands/agents.rs::ALLOWED_DOT_ENTRIES`.
3. **Per-agent opt-in** — the agent manifest may declare `initial_files_allow_hidden: [".mytool", ".otherproj"]` to extend the allowlist for project-specific tooling. Each entry must be a single hidden basename (starts with `.`, contains no `/` or `\\`, and is not `.` or `..`). Entries that match the hard-deny floor are silently filtered out of the opt-in. The `initial_files_allow_hidden` field is consumed locally and stripped from the upload payload before the server import call.

The hard-deny floor is checked on **every** path component, not just the root. So `.github/.env` is still rejected even though `.github` is allowlisted; `.claude/.ssh/config` is rejected even if a user adds `.claude` opt-ins. A skipped hidden path emits a `Warning:` to stderr identifying the rejected path and the built-in allowlist. Symlinks pointing outside the base directory are skipped regardless of policy. See `specs/threat-model.md` (TM-FS-009) for the security rationale.

### `everruns sessions`

Session management.

- `create [--harness <id|name>] [--agent <id>] [--agent-identity <id>] [--title <t>] [--locale <tag>] [--model <m>] [--system-prompt <s>] [--tag <t>] [--capability <ref[=json]>] [--hint <key=json>] [--hints-json <json>] [--network-allow <pattern>] [--network-block <pattern>] [--max-iterations <n>] [--budget-limit <[currency:]limit>] [--budget-soft-limit <[currency:]limit>]`
  - `--capability` is repeatable. Format: `REF` or `REF=JSON_CONFIG`. The CLI sends these as session-level capabilities additive to the agent and harness.
  - `--hint` is repeatable. Format: `KEY=JSON_VALUE`. `--hints-json` accepts a JSON object. Duplicate hint keys are rejected.
  - `--network-allow` and `--network-block` are repeatable network access patterns. See [`network-access.md`](network-access.md).
  - `--agent-identity` sets the resident agent identity for unattended/background execution. See [`agent-identities.md`](agent-identities.md).
  - `--max-iterations` must be greater than zero.
  - `--budget-limit` is repeatable. Format: `[CURRENCY:]LIMIT`. Currency defaults to `usd`. Multiple limits stack (most restrictive wins). Examples:
    - `--budget-limit 10` — $10 USD hard limit
    - `--budget-limit usd:10 --budget-soft-limit usd:8` — $10 hard, $8 soft pause
    - `--budget-limit tokens:2000000` — 2M token limit
    - `--budget-limit usd:10 --budget-limit tokens:2000000` — both limits, whichever hits first
- `list`
- `get <id>`
- `watch <id>` — stream session events in real time via SSE (like `kubectl logs -f`). Text mode: status/lifecycle events go to stderr, assistant message content goes to stdout (pipeable). JSON mode: each event as a JSON object to stdout. Exits cleanly on Ctrl+C.
- `export <id> [--output <file>]` — export session messages as JSONL
- `search <query>` — full-text search over sessions
- `delete <id>` — delete a session
- `cancel <id>` — cancel a running session
- `pin <id>` / `unpin <id>` — pin/unpin a session
- `resume <id>` — resume a paused session (re-activates exhausted budgets)
- `secrets <id> --secret KEY=VALUE` — set encrypted session-scoped secrets (repeatable)
- `budgets <id>` — list budgets attached to a session
- `budget-check <id>` — check whether a session is within budget

### `everruns budgets`

Spend-budget management (org/agent/session caps in a currency). Wraps the SDK `budgets()` client.

- `create --subject-type <t> --subject-id <id> [--currency <c>] --limit <n> [--soft-limit <n>]`
- `list [--subject-type <t>] [--subject-id <id>]`
- `get <id>`
- `update <id> [--limit <n>] [--soft-limit <n> | --clear-soft-limit] [--status <s>]`
- `delete <id>` — soft delete
- `top-up <id> --amount <n> [--description <d>]`
- `ledger <id> [--limit <n>] [--offset <n>]`
- `check <id>`

### `everruns chat`

Send message and poll for response.

- `chat --session <id> "<message>" [--timeout <s>] [--no-stream]`
- Polls `/v1/sessions/{id}/events` every 500ms until `turn.completed` or timeout
- No timeout by default (waits indefinitely); use `--timeout <s>` to set a limit

### `everruns capabilities`

List platform capabilities.

- `capabilities [--status available|coming_soon|all]`
- `list [--status available|coming_soon|all]`

### `everruns files`

Session filesystem operations — sync, push, pull, list, plus one-shot ops (cat, write, rm, mkdir, mv, cp, grep, stat). See [Files](#files) section below.

---

## Files

### Design Decisions

#### Decision 1: CLI Subcommand under `files`

**Chosen:** `everruns files sync --session <id> [local-dir]`
**Rationale:** Groups all file operations (sync, push, pull, ls) under one noun. `sync` is the long-running watch command; `push`/`pull` are one-shot bulk transfers.

#### Decision 2: Polling-based Change Detection

**Chosen:** Poll local filesystem with `notify` (inotify/FSEvents/kqueue) + poll remote via periodic `GET /fs/?recursive=true` with `If-Modified-Since` semantics (compare `updated_at`).
**Alternatives considered:**
- WebSocket-based push from server: Requires new server endpoint; SSE is unidirectional and event types don't include file-change events today.
- Pure polling both sides: Higher latency, more API calls.
**Rationale:** `notify` gives near-instant local detection. Remote polling at 2-5s intervals is acceptable for MVP. Server can add file-change SSE events later to eliminate remote polling.

#### Decision 3: Conflict Resolution — Last-Write-Wins with Warning

**Chosen:** If both sides changed the same file since last sync, apply last-write-wins and print a warning. Optionally `--conflict=ask` to prompt user.
**Alternatives considered:**
- Three-way merge: Complex, error-prone for binary files.
- Always-local-wins / always-remote-wins: Too aggressive.
**Rationale:** Conflicts are rare in practice (user edits locally, agent edits remotely, typically different files). Warning + configurable strategy covers edge cases without over-engineering.

#### Decision 4: `.everrunsignore` + Sensible Defaults

**Chosen:** Respect `.gitignore` patterns by default (via `ignore` crate). Additional `.everrunsignore` file for sync-specific exclusions. Always exclude: `.git/`, `node_modules/`, `target/`, `__pycache__/`, `.env`.
**Rationale:** Prevents syncing build artifacts and secrets. Aligns with developer expectations.

#### Decision 5: Incremental Sync via Content Hashing

**Chosen:** Track `sha256` content hashes locally in `<local-dir>/.everruns-sync/state.json`. Only upload/download when hash differs. The `.everruns-sync/` directory is always excluded from syncing.
**Rationale:** Avoids redundant transfers. The session filesystem already returns `content_hash` on reads.

### File Commands

#### `everruns files sync`

Long-running bidirectional watch.

```
everruns files sync --session <session_id> [local-dir]
  --session, -s     Session ID (required)
  --interval        Remote poll interval in seconds (default: 3)
  --conflict        Conflict strategy: last-write-wins | local-wins | remote-wins (default: last-write-wins)
  --exclude         Additional exclude patterns (repeatable)
  --no-gitignore    Don't read .gitignore
  --dry-run         Show what would sync without making changes
  --delete          Delete files on one side when deleted on the other (default: false)
  --verbose, -v     Show every file operation
```

**Behavior:**
1. On start: full reconciliation (compare both sides, sync differences)
2. Watch local via `notify` crate → upload changes to remote
3. Poll remote every `--interval` seconds → download changes to local
4. Print summary line on each sync cycle (e.g., `↑2 ↓1 files synced`)
5. Ctrl+C: graceful shutdown, print final stats

#### `everruns files push`

One-shot upload local → remote.

```
everruns files push --session <session_id> [local-dir]
  --session, -s     Session ID (required)
  --delete          Delete remote files not present locally (default: false)
  --dry-run         Show what would be pushed
```

#### `everruns files pull`

One-shot download remote → local.

```
everruns files pull --session <session_id> [local-dir]
  --session, -s     Session ID (required)
  --delete          Delete local files not present remotely (default: false)
  --dry-run         Show what would be pulled
```

#### `everruns files ls`

List remote session files.

```
everruns files ls --session <session_id> [path]
  --session, -s     Session ID (required)
  --recursive, -r   List recursively
  --long, -l        Show size, dates
```

#### One-shot file operations

These commands are backed directly by the SDK's typed `session_files()` client (not `RemoteClient`):

```
everruns files cat   --session <id> <path> [--output <file>]   # print/save a file's content
everruns files write --session <id> <path> [--from <file>] [--readonly]  # upload from file or stdin
everruns files rm    --session <id> <path> [--recursive]
everruns files mkdir --session <id> <path>
everruns files mv    --session <id> <src> <dest>
everruns files cp    --session <id> <src> <dest>
everruns files grep  --session <id> <pattern> [--path <glob>]   # prints path:line:text
everruns files stat  --session <id> <path>
```

`sync`/`push`/`pull` still use `RemoteClient` (raw reqwest) because they rely on the server's per-entry `content_hash` for change detection, which the SDK's `FileInfo`/`SessionFile` models do not expose.

### Sync State

Stored in `<local-dir>/.everruns-sync/state.json`:

```json
{
  "session_id": "ses_xxx",
  "last_sync": "2026-03-20T12:00:00Z",
  "files": {
    "src/main.rs": {
      "local_hash": "sha256:abc...",
      "remote_hash": "sha256:abc...",
      "local_mtime": "2026-03-20T12:00:00Z",
      "remote_updated_at": "2026-03-20T12:00:00Z"
    }
  }
}
```

### Sync Algorithm

```
for each file in (local ∪ remote):
  local_changed  = local_hash  != state.files[path].local_hash
  remote_changed = remote_hash != state.files[path].remote_hash

  if !local_changed && !remote_changed → skip
  if local_changed  && !remote_changed → upload to remote
  if !local_changed && remote_changed  → download to local
  if local_changed  && remote_changed  → apply conflict strategy
  if file only on local  → upload (or delete local if --delete and was previously synced)
  if file only on remote → download (or delete remote if --delete and was previously synced)
```

### Wire Protocol

Uses existing session filesystem REST API:

- **List remote:** `GET /v1/sessions/{id}/fs/?recursive=true`
- **Read file:** `GET /v1/sessions/{id}/fs/{path}`
- **Create file:** `POST /v1/sessions/{id}/fs/{path}` with `{ "content": "...", "encoding": "text"|"base64" }`
- **Update file:** `PUT /v1/sessions/{id}/fs/{path}` with `{ "content": "...", "encoding": "text"|"base64" }`
- **Delete file:** `DELETE /v1/sessions/{id}/fs/{path}`
- **Create dir:** `POST /v1/sessions/{id}/fs/{path}` with `{ "is_directory": true }`

Binary detection: same as server — null bytes in first 8KB → base64.

### Dependencies (new for CLI crate)

- `notify` — cross-platform filesystem watcher
- `ignore` — gitignore-compatible pattern matching
- `sha2` — content hashing
- `base64` — binary encoding (may already be transitive)
- `chrono` — timestamp handling
- `indicatif` — progress bars for push/pull (optional)
