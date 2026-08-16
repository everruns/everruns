---
type: Specification
title: "CLI Specification"
description: "CLI specification."
tags:
  - everruns
  - foundations
---
# CLI Specification

## Overview

`everruns`, command-line interface for the Everruns platform. Manages agents, sessions, chat, and file sync.

**Crate:** `crates/cli/`

**Global Flags:**
- `-o, --output`, Output format: `text` (default), `json`, `yaml`
- `-q, --quiet`, Suppress non-essential output
- `--profile <name>`, Credential profile (default: `default`)

**Configuration (precedence order):**
1. CLI flags (`--api-key`, `--api-url`)
2. Environment variables (`EVERRUNS_API_KEY`, `EVERRUNS_API_URL`)
3. Credential file (`<config_dir>/everruns/credentials.json`, Linux: `~/.config/everruns/`, macOS: `~/Library/Application Support/everruns/`)

**Credential File:**
- Multi-profile support: `{ "profiles": { "default": { "api_url", "api_key", "org_id" } }, "current_profile": "default" }`
- File permissions: `0600` on Unix
- Managed by `everruns login` / `everruns logout`

## Commands

### `everruns login`

Interactive authentication. Uses localhost HTTP callback OAuth flow.

- `login`, Open browser for OAuth login, receive a personal access token via localhost callback
- `login --token`, Paste a personal access token directly (headless/SSH fallback)

Flow: CLI → `POST /v1/auth/cli/start` (returns `auth_url` + a CSRF `state` nonce) → open browser → user logs in → server redirects to `localhost:{port}/callback?code=...&state=...` → CLI rejects the callback unless `state` matches the value from `/cli/start` (CSRF binding) → CLI calls `POST /v1/auth/cli/exchange` → receives a personal access token + user info + orgs → interactive org selection → stores in credential file.

### `everruns logout`

Remove stored credentials for current profile.

### `everruns status`

Show current user, API URL, org, and masked personal access token.

### `everruns orgs`

Organization management.

- `orgs`, List organizations (marks current with `*`)
- `orgs select`, Interactive org picker

### `everruns agents`

Agent CRUD. Create from TOML/YAML/JSON/Markdown files or CLI flags.

- `create --file <path> [--initial-files-dir <dir>] [--writable]`, send file to server import API; the CLI normalizes TOML to JSON and otherwise forwards the file as-is. Upserts when `id:` is present in the definition. If `--file` is omitted and `./agent.toml` exists, the CLI uses it automatically unless inline creation flags are present. `--initial-files-dir` recursively collects non-hidden text files from the directory and injects them as read-only `initial_files`. `--writable` makes collected files writable. Alternatively, `initial_files` can be specified directly in the definition as a list of relative paths, each entry is resolved relative to the agent file's parent directory, directories are walked recursively, and files are collected with the same security rules as `--initial-files-dir`.
- `create --name <n> --system-prompt <s> [--description <d>] [--model <m>] [--harness <id|name>] [--tag <t>]`, create from CLI flags. `--harness` (`-H`) accepts a harness id (`harness_<32-hex>`) or a name (e.g. `generic`); a strict id is sent as `harness_id`, anything else as `harness_name`. Omitting it defaults to the org's built-in `generic` harness. On the `--file` path the harness comes from the definition's `harness_id` / `harness_name` key instead.
- `update --file <path> [--initial-files-dir <dir>] [--writable]`, send file to server import API; TOML is normalized client-side, and `./agent.toml` is used automatically when `--file` is omitted, no inline update flags are present, and no positional `<id>` is provided. Requires `id:` in the definition for upsert. `--initial-files-dir` and definition `initial_files` work the same as in create.
- `update <id> --name <n> --system-prompt <s> [--description <d>] [--model <m>] [--harness <id|name>] [--tag <t>]`, update from CLI flags. `--harness` (`-H`) uses the same id-vs-name detection as create.
- `list`
- `get <id>`
- `delete <id>` (soft archive)

#### Initial Files Hidden Path Policy

`initial_files` collection (whether from `--initial-files-dir` or definition globs) gates hidden (dot-prefixed) path components to prevent accidental upload of host secrets. The policy has three layers, evaluated per path component:

1. **Hard-deny floor**: known credential / version-control / shell-history paths are never uploaded. Examples: `.env`, `.env.local`, `.envrc`, `.ssh`, `.gnupg`, `.aws`, `.azure`, `.gcloud`, `.kube`, `.docker`, `.npmrc`, `.yarnrc`, `.pypirc`, `.netrc`, `.cargo`, `.git`, `.hg`, `.svn`, `.bash_history`, `.zsh_history`, `.python_history`, `.node_repl_history`. The full list lives in `crates/cli/src/commands/agents.rs::DENIED_DOT_ENTRIES`. The hard-deny floor cannot be bypassed by user opt-in.
2. **Built-in allowlist**: common dev-ecosystem assets are allowed by default. Exact basenames: `.agents`, `.github`, `.vscode`, `.claude`, `.cursor`, `.mcp.json`, `.gitignore`, `.gitattributes`, `.editorconfig`, `.prettierrc`, `.prettierrc.json`, `.prettierrc.yaml`, `.prettierrc.yml`, `.prettierrc.js`, `.prettierrc.cjs`, `.prettierrc.mjs`, `.eslintrc`, `.eslintrc.json`, `.eslintrc.yaml`, `.eslintrc.yml`, `.eslintrc.js`, `.eslintrc.cjs`, `.eslintignore`, `.nvmrc`, `.node-version`, `.python-version`, `.tool-versions`, `.dockerignore`, `.rubocop.yml`. The authoritative list lives in `crates/cli/src/commands/agents.rs::ALLOWED_DOT_ENTRIES`.
3. **Per-agent opt-in**: the agent manifest may declare `initial_files_allow_hidden: [".mytool", ".otherproj"]` to extend the allowlist for project-specific tooling. Each entry must be a single hidden basename (starts with `.`, contains no `/` or `\\`, and is not `.` or `..`). Entries that match the hard-deny floor are silently filtered out of the opt-in. The `initial_files_allow_hidden` field is consumed locally and stripped from the upload payload before the server import call.

The hard-deny floor is checked on **every** path component, not just the root. So `.github/.env` is still rejected even though `.github` is allowlisted; `.claude/.ssh/config` is rejected even if a user adds `.claude` opt-ins. A skipped hidden path emits a `Warning:` to stderr identifying the rejected path and the built-in allowlist. Symlinks pointing outside the base directory are skipped regardless of policy. See `knowledge/security/threat-model.md` (TM-FS-009) for the security rationale.

### `everruns sessions`

Session management.

- `create [--harness <id|name>] [--agent <id|name>] [--agent-identity <id>] [--title <t>] [--locale <tag>] [--model <m>] [--system-prompt <s>] [--tag <t>] [--capability <ref[=json]>] [--hint <key=json>] [--hints-json <json>] [--secret <KEY=VALUE>] [--network-allow <pattern>] [--network-block <pattern>] [--max-iterations <n>] [--budget-limit <[currency:]limit>] [--budget-soft-limit <[currency:]limit>]`
  - `--agent` accepts an agent id (`agent_…`) or a name; a bare name resolves server-side. When `--agent` is given and `--harness` is omitted, the session runs on the agent's harness (agent-first). An explicit `--harness` still overrides it.
  - `--capability` is repeatable. Format: `REF` or `REF=JSON_CONFIG`. The CLI sends these as session-level capabilities additive to the agent and harness.
  - `--secret` is repeatable. Format: `KEY=VALUE`. Injects session-scoped secrets at creation time (not tied to a user connection provider).
  - `--hint` is repeatable. Format: `KEY=JSON_VALUE`. `--hints-json` accepts a JSON object. Duplicate hint keys are rejected.
  - `--network-allow` and `--network-block` are repeatable network access patterns. See [`network-access.md`](../operations/network-access.md).
  - `--agent-identity` sets the resident agent identity for unattended/background execution. See [`agent-identities.md`](../runtime-resources/agent-identities.md).
  - `--max-iterations` must be greater than zero.
  - `--budget-limit` is repeatable. Format: `[CURRENCY:]LIMIT`. Currency defaults to `usd`. Multiple limits stack (most restrictive wins). Examples:
    - `--budget-limit 10`, $10 USD hard limit
    - `--budget-limit usd:10 --budget-soft-limit usd:8`, $10 hard, $8 soft pause
    - `--budget-limit tokens:2000000`, 2M token limit
    - `--budget-limit usd:10 --budget-limit tokens:2000000`, both limits, whichever hits first
- `list`
- `get <id>`
- `watch <id>`, stream session events in real time via SSE (like `kubectl logs -f`). Text mode: status/lifecycle events go to stderr, assistant message content goes to stdout (pipeable). JSON mode: each event as a JSON object to stdout. Exits cleanly on Ctrl+C.
- `export <id> [-o <path>] [--format jsonl|atif]`, export session messages to a file (`-o`/`--output`) or stdout. `--format` defaults to `jsonl` (one message per line); `atif` emits an ATIF trajectory.

### `everruns triggers`

Manage schedule triggers scoped to an agent with `--agent <id>`.

- `list`, list the agent's active triggers. Text output renders common cron schedules as a human-readable cadence with timezone; JSON and YAML retain the API response.
- `create --cron <expression> --message <text> [--timezone <iana>] [--session-mode shared-session|session-per-invocation] [--disabled]`
- `update <trigger-id> [--cron <expression>] [--message <text>] [--timezone <iana>] [--session-mode shared-session|session-per-invocation]`
- `enable <trigger-id>`
- `disable <trigger-id>`
- `run-now <trigger-id>`, fire one invocation immediately.

Examples:

```bash
everruns triggers --agent agent_... create \
  --cron '30 * * * *' \
  --timezone America/Chicago \
  --session-mode session-per-invocation \
  --message 'Prepare the hourly report'
everruns triggers --agent agent_... list
everruns triggers --agent agent_... run-now trg_...
```

### `everruns participants`

Manage participants scoped to a session with `--session <id>`.

- `list`, list active and past participants, including host/member role and leave status.
- `add --agent <agent-id>` (alias: `invite`), invite an agent as a member.
- `remove <participant-id>`, mark an active member as having left. The session host cannot be removed.

```bash
everruns participants --session session_... add --agent agent_...
everruns participants --session session_... list
everruns participants --session session_... remove part_...
```

### `everruns chat`

Send message and poll for response.

- `chat --session <id> "<message>" [--timeout <s>] [--no-stream]`
- Polls `/v1/sessions/{id}/events` every 500ms until `turn.completed` or timeout
- No timeout by default (waits indefinitely); use `--timeout <s>` to set a limit

### `everruns capabilities`

List platform capabilities.

- `capabilities [--status available|coming_soon|all]`
- `list [--status available|coming_soon|all]`

### `everruns connections`

Manage per-provider API-key connections (e.g. `daytona`, `brave_search`, `browserless`, `deno`, `sprites`).

- `set <provider> [--stdin]`, set an API key for a provider (read the key from stdin with `--stdin`).
- `list`, list connected providers.
- `remove <provider>`, remove a connection.

### `everruns files`

Session filesystem operations, sync, push, pull, list. See [Files](#files) section below.

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

#### Decision 3: Conflict Resolution, Last-Write-Wins with Warning

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

Binary detection: same as server, null bytes in first 8KB → base64.

### Dependencies (new for CLI crate)

- `notify`, cross-platform filesystem watcher
- `ignore`, gitignore-compatible pattern matching
- `sha2`, content hashing
- `base64`, binary encoding (may already be transitive)
- `chrono`, timestamp handling
- `indicatif`, progress bars for push/pull (optional)
