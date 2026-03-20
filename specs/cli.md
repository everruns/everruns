# CLI Specification

## Overview

`everruns` — command-line interface for the Everruns platform. Manages agents, sessions, chat, and file sync.

**Crate:** `crates/cli/`

**Global Flags:**
- `-o, --output` — Output format: `text` (default), `json`, `yaml`
- `-q, --quiet` — Suppress non-essential output

**Configuration:**
- `EVERRUNS_API_KEY` — API authentication token
- `EVERRUNS_API_URL` — Base URL (default: `https://app.everruns.com/api`)

## Commands

### `everruns agents`

Agent CRUD. Create from YAML/JSON/Markdown (YAML front matter + body = system prompt).

- `create --file <path>` | `--name <n> --system-prompt <s>`
- `list`
- `get <id>`
- `delete <id>` (soft archive)

### `everruns sessions`

Session management.

- `create --harness <id> [--agent <id>] [--title <t>] [--model <m>]`
- `list`
- `get <id>`

### `everruns chat`

Send message and poll for response.

- `chat --session <id> "<message>" [--timeout <s>] [--no-stream]`
- Polls `/v1/sessions/{id}/events` every 500ms until `turn.completed` or timeout (default 300s)

### `everruns capabilities`

List platform capabilities.

- `list [--status available|coming_soon|all]`

### `everruns files`

Session filesystem operations — sync, push, pull, list. See [Files](#files) section below.

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

#### Decision 4: `.syncignore` + Sensible Defaults

**Chosen:** Respect `.gitignore` patterns by default (via `ignore` crate). Additional `.syncignore` file for sync-specific exclusions. Always exclude: `.git/`, `node_modules/`, `target/`, `__pycache__/`, `.env`.
**Rationale:** Prevents syncing build artifacts and secrets. Aligns with developer expectations.

#### Decision 5: Incremental Sync via Content Hashing

**Chosen:** Track `sha256` content hashes locally in `.sync-state.json` (in sync metadata dir). Only upload/download when hash differs.
**Rationale:** Avoids redundant transfers. The session filesystem already returns `content_hash` on reads.

### File Commands

#### `everruns files sync`

Long-running bidirectional watch.

```
everruns files sync --session <session_id> [local-dir]
  --session, -s     Session ID (required)
  --interval        Remote poll interval in seconds (default: 3)
  --conflict        Conflict strategy: last-write-wins | local-wins | remote-wins | ask (default: last-write-wins)
  --exclude         Additional exclude patterns (repeatable)
  --include         Override excludes for specific patterns (repeatable)
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
everruns files push --session <session_id> [local-dir] [-- paths...]
  --session, -s     Session ID (required)
  --delete          Delete remote files not present locally (default: false)
  --dry-run         Show what would be pushed
```

#### `everruns files pull`

One-shot download remote → local.

```
everruns files pull --session <session_id> [local-dir] [-- paths...]
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

Binary detection: same as server — null bytes in first 8KB → base64.

### Future Enhancements

1. **Server-side file change events** via SSE (`file.created`, `file.updated`, `file.deleted`) to eliminate remote polling
2. **Delta sync** using content-defined chunking for large files
3. **Multi-session sync** (fan-out from one local dir to multiple sessions)
4. **Selective path sync** (`everruns files sync --session ses_xxx ./src` to sync only `src/`)
5. **Integration with `everruns chat`** — auto-sync while chatting

### Dependencies (new for CLI crate)

- `notify` — cross-platform filesystem watcher
- `ignore` — gitignore-compatible pattern matching
- `sha2` — content hashing
- `base64` — binary encoding (may already be transitive)
- `chrono` — timestamp handling
- `indicatif` — progress bars for push/pull (optional)
