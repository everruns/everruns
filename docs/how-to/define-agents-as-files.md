---
title: Define agents as files
description: Load, validate, compare, import, export and serve portable agents with instructions, files, skills, MCP servers and channel descriptions.
appliesTo: [framework, platform, cloud]
---

An agent package describes authored behavior and assets. The same definition
works in the UI, API, CLI, Platform Chat, MCP, Framework and serve. Use a single
Markdown file for simple agents or an `agent.toml` folder for assets. ZIP is the
transport format for complete folders.

## Keep simple Markdown agents

Existing Markdown with YAML front matter remains supported. Its body becomes
**instructions**. The old `system_prompt` field is accepted as an input alias;
new exports use `instructions`. Conflicting values fail validation.

```markdown
---
name: dad-jokes
capabilities:
  - current_time
---
Tell one short, family-friendly dad joke when asked.
```

Unversioned legacy imports can still use `id`, `default_model_id` and
`harness_id`. New packages and every export omit these resource IDs. Use
`--target dad-jokes` to update an existing agent explicitly.

## A folder with good defaults

```text
triage/
├── agent.toml
├── instructions.md
├── files/
│   └── runbook.md
└── skills/
    └── investigate/
        ├── SKILL.md
        └── scripts/check.py
```

```toml
schema_version = 1
name = "triage"
display_name = "Triage assistant"
capabilities = ["session_file_system", "current_time"]
max_iterations = 20
parallel_tool_calls = false

[channels.chat]
type = "ag_ui"
[channels.chat.config]
tool_visibility = "generic"
generic_tool_text = "Investigating…"
rate_limit_per_minute = 30
```

A folder contains exactly one `agent.toml`, `agent.md`, `agent.yaml`,
`agent.yml` or `agent.json`. The defaults are:

- `instructions.md` supplies instructions when none are inline.
- `files/` supplies initial workspace files, mounted at `/`, read-only.
- `skills/` supplies direct skill subfolders, including scripts, references and
  binary assets. Each `SKILL.md` must have valid front matter and a name matching
  its directory. Skills mount at `/.agents/skills/`; the `skills` capability is added.
- The Platform resolves its default model and harness when omitted. Framework
  applications bind a model explicitly. Serve uses its simulator when omitted.
- Channels default to disabled. `enabled = true` requests a draft channel; publication and credentials are destination bindings.

TOML, YAML and JSON share one schema. Capability strings select default config;
`{ ref = "current_time", config = {} }` supplies explicit config. See the
[package reference](#package-reference) below for optional fields.

## Describe MCP dependencies

Use a destination catalog name when the host owns authentication and installation:

```toml
[mcpServers.issues]
use = "catalog:linear"
actsAs = "user"
```

An inline server can declare explicit credential requirements:

```toml
[mcpServers.docs]
type = "http"
url = "https://example.com/mcp"
[mcpServers.docs.headers]
Authorization = "${DOCS_AUTHORIZATION}"
```

The Framework resolves that requirement only through `package.bind_mcp(...)`.
Bind a catalog requirement with a same-named `AgentBuilder::mcp_server` before
applying the package. Serve explicitly resolves named environment requirements
at startup. Platform imports require destination catalog bindings for credentials.

## Validate, compare and import

```bash
# Offline; no platform credentials required.
everruns agents validate ./triage
everruns agents diff ./triage --against ./triage-before

# Check destination models, harnesses, capabilities, MCP catalog and channels.
everruns agents validate ./triage --remote
everruns agents diff ./triage --target triage

# Create, or explicitly update by name.
everruns agents import ./triage
everruns agents import ./triage --target triage

# Full package export; does not overwrite existing output files.
everruns agents export triage --format folder --out ./triage-export
everruns agents export triage --format zip --out ./triage.zip
everruns agents export triage --format markdown --out ./triage.md
```

`agents create --file agent.toml` remains supported, including the existing
`--initial-files-dir` and `--writable` options. New file workflows use
`agents import`. Inline creation accepts `--instructions`; `--system-prompt`
remains an alias.

Diffs compare normalized authored values: capability and tag ordering is ignored;
file changes show SHA-256, size and read-only status instead of dumping contents.
Remote diffs preserve omitted channel bindings, matching import's channel-upsert
behavior. Diff and validate never apply changes. Import replaces the agent's
configuration and upserts declared channel configuration. New channels default to disabled; `enabled = true` creates a draft;
existing enablement and credential bindings remain at the destination, and omitted
channels survive.

## UI and HTTP API

In **Agents → Import**, select a Markdown, TOML, YAML, JSON or ZIP file. Review
validation, choose a new agent or an existing destination, then apply. Existing
agents show a diff before updating. Export offers a complete ZIP or Markdown.

The HTTP import endpoint accepts raw text or ZIP bytes. Folder references must
be materialized by a local loader or supplied in ZIP; the server never reads
paths from its own disk.

```bash
curl -X POST "$EVERRUNS_API_URL/v1/agents/import?format=zip" \
  -H "Authorization: Bearer $EVERRUNS_API_KEY" \
  -H "Content-Type: application/zip" --data-binary @triage.zip

curl "$EVERRUNS_API_URL/v1/agents/triage/export?format=zip" \
  -H "Authorization: Bearer $EVERRUNS_API_KEY" -o triage.zip
```

`POST /v1/agents/validate` and `POST /v1/agents/diff` accept the same bytes or an
envelope `{ "content": "…", "format": "toml", "target": "triage" }`.
Validation returns `valid` and field-addressed `diagnostics`. Diff requires
`target` and returns `changed` and `changes`. Export formats are `markdown`
(default), `toml`, `yaml`, `json` and `zip`.

## Platform Chat and MCP

The catalog exposes `import_agent`, `export_agent`, `validate_agent_package`
and `diff_agent_package`. Use the same CLI verbs inside Platform Chat:

```bash
everruns agents validate /agents/triage
everruns agents diff /agents/triage --target triage
everruns agents import /agents/triage --target triage
everruns agents export triage --format zip --out /exports/triage.zip
```

`file` and `out` refer only to the current session workspace. Reads and writes
use its existing tenant, permission and private-memory checks. Export creates a
new artifact and fails if that path already exists. MCP clients without a current
session send materialized `content`; export without `out` returns the portable
manifest. `--content` also accepts inline text in Platform Chat.

## Framework: load and run

```rust
use everruns::{AgentPackage, Engine, Model, PackageFormat};

let package = AgentPackage::load("./triage")?;
let agent = package.builder()?
    .model(Model::simulated("Ready to triage."))
    .build()?;
let session = package.create(&Engine::new(), agent)?;
let result = session.send_and_wait("Help with an issue.").await?;
assert!(result.success);

package.write_folder("./triage-export")?;
let markdown = package.to_string(PackageFormat::Markdown)?;
```

Disk loading and folder export are enabled by the Framework's default
`agent-package-fs` feature. Applications using `default-features = false` must
select it explicitly. In-memory parsing, ZIP and diffs remain available without
disk access. The shared codec lives in `everruns-core::agent_package`, behind the
core `agent-package` feature; core's default build is unchanged.

For live inference, bind the provider in application code. MCP requires the
`mcp` feature; stdio requires `mcp-stdio`. Resolve `${ENV_NAME}` values explicitly
with `package.bind_mcp(|name| ...)` before applying the package. Custom tools
must be bound through `.tool(...)` before `package.apply_to(builder)`; names and
schemas must match. Unknown capabilities fail with a dependency diagnostic.

`Agent::to_package()` exports configuration, files and custom function-tool
schemas. Closures, lifecycle hooks, approval handlers and code-defined
capabilities require host code; they cannot be transferred as executable code.
Packages preserve channel descriptions; Framework applications bind
transports separately. Platform environment declarations need explicit session
bindings and are rejected by the generic package builder.

`package.create` resolves the standard `base`, `conversation`, `worker-base` and
`worker` harnesses. For custom harnesses, use `Engine::create(agent).harness(...)`.

## Serve a file or folder

```rust
serve::start(
    serve::App::builder().agent_package("./triage").build()
).await
```

Combine `.discover()` with `.agent_package(...)` to bind registered custom
tools. Assets are loaded and frozen at app construction; restart to apply edits.
Alternatively, `AppBuilder::package(AgentPackage::from_zip(include_bytes!(...))?)`
embeds a package in the binary. The serve build manifest includes the declaration
and asset contents so edits change its build ID. Channel descriptions do not
register arbitrary network handlers; configure serve channel transports separately.

## Package reference

| Field | Meaning and default |
| --- | --- |
| `schema_version` | `1`; unknown versions fail |
| `name` | Lowercase letters/digits/hyphens; stable address, not a platform ID |
| `instructions` / `instructions_file` | Inline instructions or a package-relative file; mutually exclusive |
| `model` | `{ provider = "openai", model = "…" }`; destination binds an enabled provider/model pair |
| `harness` | Destination harness name; omitted uses the host default |
| `capabilities` | Stable capability names or `ref`/`config` objects |
| `initial_files` | Inline `path`/`content`/`encoding`/`is_readonly`, relative paths/globs, or `source` mappings |
| `skills` | Relative directories containing skill subfolders; folder default `skills/` |
| `mcpServers` | Existing scoped MCP schema; named `use = "catalog:name"` references or inline HTTP/stdio settings |
| `channels` | Named descriptions with `type`, `config`, and optional `enabled` (default `false`) |
| `network_access` | Existing network policy; host enforcement still applies |
| `max_iterations`, `parallel_tool_calls` | Existing execution settings; omitted uses runtime defaults |
| `tools` | Client-side tool schemas; Framework requires matching host handlers |
| `display_name`, `description`, `tags` | Display metadata |
| `intro_markdown`, `short_description`, `starters`, `environments` | Platform presentation and environment declarations |

Map a directory explicitly, for example to `/data`, while retaining read-only files:

```toml
initial_files = [{ source = "fixtures", path = "/data" }]
```

`encoding` defaults to `text`; binary inline content uses `base64`.
`is_readonly` defaults to `true`. Relative sources stay inside the package;
absolute paths, traversal, symlinks, duplicate destinations, invalid Base64 and
unknown manifest fields are rejected. Hidden credential files are skipped by
folder/glob collection; explicit hidden credential sources fail. Maximums:
10 MiB package, 100 initial files, 1 MiB per asset, 5 MiB decoded initial content,
and 32 channel declarations.
Native folder scans also cap nesting at 32 levels and visited entries at 1,024.
Remote validation also enforces the destination Platform’s input and resource limits.

Credential-bearing MCP headers/env must use placeholders. The Platform requires
catalog bindings instead of reading worker environment variables. Literal MCP
credentials are replaced with requirements on export. Channel credentials and
resource IDs are omitted. Installed plugin IDs cannot be exported; describe their
portable MCP or declarative capability requirements instead. Treat packaged file
contents as authored data and review them before sharing.

The Platform creates `ag_ui`, `public_chat`, `fcp` and `slack` channels from packages.
They default to disabled; explicit `enabled = true` creates a draft that must be
published through the channel API. Imports preserve activation of existing channels
and preflight permission to modify live configuration before changing the agent. Other known channel types can be described for hosts,
but Platform validation rejects them until bound through their dedicated channel
or trigger APIs. Public Chat also requires its feature flag. Schedules remain
trigger resources. Packages do not embed sessions, history, grants or deployment state.

## Examples and references

- [Simple Markdown and complete triage folder](https://github.com/everruns/everruns/tree/main/examples/agent-packages)
- [Runnable Framework example](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/file_agent.rs)
- [Shared package API](https://docs.rs/everruns-core/latest/everruns_core/agent_package/)
- [REST API reference](/api/)
- [CLI](/features/cli/)
- [Framework agents](/framework/agents/) and [serve](/framework/serve/)
- [MCP configuration](/features/mcp/)
