# Tool narration

Every tool call shown in a transcript or timeline carries a human-readable
narration line ("Read AGENTS.md", "Searched tools: router"). everruns authors
this narration in the backend so that **downstream clients do not need
per-tool narration code**. A client should be able to render with nothing more
than:

```rust
data.narration.unwrap_or(display_name_or_tool_name)
```

The narration system lives in [`crates/core/src/tool_narration.rs`](../crates/core/src/tool_narration.rs)
and is emitted on tool events (see [`specs/events.md`](events.md) — the
`narration` field). Narration is a backend-authored, localizable string (see
[`specs/localization.md`](localization.md), `tool.narration.*` keys).

## Why this lives in everruns

Hosts and apps (e.g. Yolop) repeatedly reinvented per-tool narration for the
same common tool shapes — search, fetch, shell, config, memory, skills,
background tasks. That duplication drifts and produces inconsistent phrasing.
everruns owns neutral, argument-aware narration for **common tool names and
common argument schemas** so every consumer gets the same line for free.

Apps should treat any remaining app-side narration as a temporary safety net,
not the primary source. When an app finds itself adding custom narration for a
generally-reusable tool shape, the fix is to add a rule here, not in the app.

## Resolution order

For a given tool call and [`ToolNarrationPhase`](../crates/core/src/tool_narration.rs)
(`Started`, `Waiting`, `Completed`, `Failed`), narration resolves in order:

1. **Exact tool-name rule** — known built-in/common tool names (`read_file`,
   `bash`, `tool_search`, …) with hand-tuned phrasing and argument reads.
2. **Generic pattern rule** — name-shape and argument-shape rules that cover
   whole families (`*_search`, names containing `fetch`/`symbol`/`config`/…).
   These catch tools everruns has never seen by name.
3. **`narration_noun` operation narration** — multi-operation CRUD tools whose
   `ToolHints` set `narration_noun` and whose args carry `operation`/`action`
   (see [`specs/tool-execution.md`](tool-execution.md#narration-formatting)).
4. **Generic fallback** — `"{verb} {display_name}"` from the tool's display
   name (localized) or title-cased tool name.

Earlier rules win. A generic pattern rule must never override a more specific
exact rule. Adding an exact rule is for nicer phrasing or a different argument;
the generic rules guarantee that *un-named* tools still narrate sensibly.

## Phrasing

Neutral imperative for in-progress phases, neutral past for completed, and
"Could not …" for failed. No first-person ("I'm …"). Prefer human labels over
tool-internal names when an argument gives a better one.

| Phase | Style | Example |
|-------|-------|---------|
| `Started` / `Waiting` | imperative | `Search tools: router` |
| `Completed` | past | `Searched tools: router` |
| `Failed` | could-not | `Could not search tools: router` |

When the relevant argument is absent, drop the value and keep the bare verb
phrase (`Search tools`, `Searched tools`, `Could not search tools`).

## Argument display and redaction

Arguments shown in narration are **display values, not faithful echoes**:

- **Truncate.** queries / patterns / commands: ~48–80 chars; longer values get
  an ellipsis. File paths show the basename only.
- **URLs.** show host + path; strip the query string by default (it may carry
  tokens). Truncate long paths.
- **Never show secrets.** Fields named `token`, `api_key`, `apikey`,
  `password`, `secret`, `authorization`, or similar are never rendered, even as
  the chosen display argument — fall back to the bare verb phrase instead.
- **Never dump prompts.** Long free-text like background-agent instructions or
  full approval-action text is not shown; narrate the action, not its payload.

## Generic pattern rules

These cover families by name/argument shape so new, unseen tools narrate well.
Argument lookup tries the listed keys in order and uses the first present.

| Rule (name shape) | Args read | Example |
|-------------------|-----------|---------|
| exact `tool_search` or `*_tool_search` | `query` | Search tools: query |
| `*_search` / names containing `search` | `query`, `q`, `search`, `pattern` | Search DuckDuckGo: rust docs |
| names containing `fetch` | `url`, `uri` | Fetch URL: example.com/page |
| names containing `symbol` | `query`, `symbol`, `path` | Search symbols: Router |
| names containing `repo_map` / map+repo/path | `path` | Build repo map: src |
| `activate_skill` / `*_skill` with activate verb | `name`, `skill`, `id` | Activate skill: ship |
| config read (`get`/`read` + config) | `key` | Read config: providers.openai.model |
| config write (`set`/`update` + config) | `key` | Update config: default_model |
| memory (`remember`/`recall`/`forget`/`memory`) | `title`, `query`, `id` | Save memory: User preference |
| names containing `hook` | `id`, `name` | Validate hook: block-git |
| names containing `connector`/`connect` | `provider`, `name` | Connect provider: daytona |
| names containing `approval` | `mode` only (never the action text) | Set approval mode: protective |
| shell: `bash`, `shell`, `run_shell`, `run_command` | `command`, `commands`, `cmd` | Run shell command: cargo test |
| background task: `background_*` | `task_id`, `id`, `command` | Read background output: task-1 |

## Family catalog

The narration system commits to covering the tool families below so that the
"next tool" added to a host with one of these shapes narrates without app-side
code. Phrasing columns are `started/waiting` → `completed` → `failed`; the
short form after `/` is the no-argument fallback.

### Discovery and skills

| Tool | Args | Narration |
|------|------|-----------|
| `tool_search` | `query` | Search tools: {query} / Search tools → Searched tools: {query} → Could not search tools: {query} |
| `activate_skill` | `name` | Activate skill: {name} → Activated skill: {name} → Could not activate skill: {name} |
| `read_skill` | `name` | Read skill: {name} → Read skill: {name} → Could not read skill: {name} |
| `list_skills` | — | List skills → Listed skills → Could not list skills |
| `run_command` / `run_client_command` | `command`, `arguments` | Run command: /{command} → Ran command: /{command} → Could not run command: /{command} |

`run_command` is the generalized name for app slash-command runners (e.g.
Yolop's `run_yolop_command`). Show `/{command}`; do not dump `arguments`.

### Code intelligence and web

| Tool | Args | Narration |
|------|------|-----------|
| `repo_symbols` | `query`, `path` | Search symbols: {query} / List symbols → Searched symbols: {query} / Listed symbols → Could not search symbols: {query} |
| `repo_map` | `path` | Build repo map: {path} → Built repo map: {path} → Could not build repo map: {path} |
| `ast_grep` | `pattern`, `language`, `path` | Search code: {pattern} → Searched code: {pattern} → Could not search code: {pattern} |
| `duckduckgo_search` | `query` | Search DuckDuckGo: {query} → Searched DuckDuckGo: {query} → Could not search DuckDuckGo: {query} |
| `web_fetch` | `url` | Fetch URL: {host/path} → Fetched URL: {host/path} → Could not fetch URL: {host/path} |

`ast_grep` is code/AST search, not file search — do not route it through the
`grep_files` ("Search files") phrasing. `duckduckgo_search` is also covered by
the generic `*_search` rule; the exact rule only sharpens the label.

### Shell and background execution

| Tool | Args | Narration |
|------|------|-----------|
| `bash` | `command` | Run shell command: {command} → Ran shell command: {command} → Shell command failed: {command} |
| `background_run` | `command` | Start background command: {command} → Started background command: {command} → Could not start background command: {command} |
| `background_output` | `task_id`, `id` | Read background output: {task_id} → Read background output: {task_id} → Could not read background output: {task_id} |
| `background_list` | — | List background tasks → Listed background tasks → Could not list background tasks |
| `background_cancel` | `task_id`, `id` | Cancel background task: {task_id} → Canceled background task: {task_id} → Could not cancel background task: {task_id} |
| `background_agent` | — (never `instruction`) | Start background agent → Started background agent → Could not start background agent |
| `write_todos` | — | Update task list → Updated task list → Could not update task list |

`bash` is covered directly even though shell execution also has provider-named
variants (`*_exec`). `background_agent` never echoes its prompt/instruction.
`write_todos` narration stays generic; clients may still render the result
specially (todo lines).

### Config and personalization

| Tool | Args | Narration |
|------|------|-----------|
| `get_config` | `key` | Read config: {key} → Read config: {key} → Could not read config: {key} |
| `set_config` | `key` | Update config: {key} → Updated config: {key} → Could not update config: {key} |
| `remember` | `title` | Save memory: {title} → Saved memory: {title} → Could not save memory: {title} |
| `recall` | `query`, `id` | Search memory: {query} / Read memory → Searched memory: {query} / Read memory → Could not search memory: {query} |
| `forget` | `id`, `title` | Delete memory: {title_or_id} → Deleted memory: {title_or_id} → Could not delete memory: {title_or_id} |

### Hooks

| Tool | Args | Narration |
|------|------|-----------|
| `list_hooks` | — | List hooks → Listed hooks → Could not list hooks |
| `validate_hook` | `id`, `spec.id` | Validate hook: {id} → Validated hook: {id} → Could not validate hook: {id} |
| `upsert_hook` | `id`, `spec.id` | Save hook: {id} → Saved hook: {id} → Could not save hook: {id} |
| `remove_hook` | `id` | Remove hook: {id} → Removed hook: {id} → Could not remove hook: {id} |

### Connectors

| Tool | Args | Narration |
|------|------|-----------|
| `list_connectors` | — | List connectors → Listed connectors → Could not list connectors |
| `get_connector` | `provider`, `name` | Read connector: {provider} → Read connector: {provider} → Could not read connector: {provider} |
| `connect` | `provider`, `name` | Connect provider: {provider} → Connected provider: {provider} → Could not connect provider: {provider} |
| `disconnect` | `provider`, `name` | Disconnect provider: {provider} → Disconnected provider: {provider} → Could not disconnect provider: {provider} |

### Approval and safety

| Tool | Args | Narration | Note |
|------|------|-----------|------|
| `record_approval` | — (never `action`) | Record approval → Recorded approval → Could not record approval | The approved action may contain sensitive detail; never echo it. |
| `set_approval_mode` | `mode` | Set approval mode: {mode} → Set approval mode: {mode} → Could not set approval mode: {mode} | The mode is safe to show. |

## Already covered (file tools)

These have everruns-owned narration today and should **not** need app-side
narration. Listed so apps can delete their copies.

| Tool / pattern | Narration |
|----------------|-----------|
| `read_file`, `session_read_file` | Read {basename} → Read {basename} → Could not read {basename} |
| `read_many_files` | Read multiple files → … → Could not read multiple files |
| `list_directory`, `list_files` | List files in {dir} → Listed files in {dir} → Could not list files in {dir} |
| `grep_files` | Search files: {pattern} → Searched files: {pattern} → Could not search files: {pattern} |
| `search`, `search_web` | Search web: {query} → Searched web: {query} → Could not search web: {query} |
| `*__search` (MCP/provider) | generic search narration |
| `write_file` | Write {basename} → Wrote {basename} → Could not write {basename} |
| `edit_file`, `replace_in_file` | Edit {basename} → Edited {basename} → Could not edit {basename} |
| `append_file` | Append {basename} → Appended {basename} → Could not append {basename} |
| `move_file` | Move {a} to {b} → Moved {a} to {b} → Could not move {a} to {b} |
| `delete_file` and remove-like file tools | Delete {basename} → Deleted {basename} → Could not delete {basename} |

## Localization

Narration is subject to backend localization
([`specs/localization.md`](localization.md)). English is the reference. New
families should be added in English first; locales that have not yet localized
a family fall back to a generic localized verb phrase rather than mixing
languages (see the Ukrainian path in `tool_narration.rs`).

## Adding a new family

1. Prefer a **generic pattern rule** if the family has a recognizable
   name/argument shape — it covers unseen siblings for free.
2. Add an **exact rule** only when the family needs sharper phrasing or a
   specific argument the generic rule cannot infer.
3. Pick the display argument carefully — apply truncation and the redaction
   list above. Never select a secret-bearing field.
4. Add unit tests in `tool_narration.rs` asserting `Started`, `Completed`, and
   `Failed` lines, including the no-argument fallback.
