---
title: Define agents as files
description: Load, validate, compare, import, export and serve portable agents with instructions, files, skills, MCP servers and channel descriptions.
appliesTo: [framework, platform, cloud]
---

An agent package describes authored behavior and assets. The same definition
works in the UI, API, CLI, Platform Chat, MCP, Framework and serve. Use an `agent.toml` folder with instructions and selected assets. ZIP is the
transport format for complete folders.

## A folder with good defaults

```text
triage/
├── agent.toml
├── instructions.md
├── runbook.md
├── data/
│   └── example.csv
└── .agents/
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
files = ["runbook.md", "data/**"]

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
- `files` explicitly selects ordinary files or globs. Paths stay relative to the
  directory root: `data/example.csv` becomes `data/example.csv` in Agent Files
  and Session Files. Files are read-only unless `is_readonly = false` is declared.
  Unselected project files are never automatically included.
- `.agents/skills/` supplies direct skill subfolders at the same relative paths,
  including scripts, references and binary assets. Each `SKILL.md` must have
  valid front matter and a name matching its directory. The `skills` capability
  is added automatically.
- `agent.toml` and `instructions.md` configure behavior; they are runtime files
  only when explicitly selected in `files`. Exports preserve runtime files that
  collide with authoring filenames as inline entries in the manifest.
- The Platform resolves its default model and harness when omitted. Framework
  applications bind a model explicitly. Serve uses its simulator when omitted.
- Channels default to disabled. `enabled = true` requests a draft channel; publication and credentials are destination bindings.

TOML, YAML and JSON share one schema. Capability strings select default config;
`{ ref = "current_time", config = {} }` supplies explicit config. See the
[format reference](/reference/agent-package/) below for optional fields.

## Paths, environments and session files

An Environment describes how execution is provisioned; a Sandbox is the live
execution resource. Both use the same relative file tree. The physical working
directory is host-specific (for example `/workspace` in a managed sandbox).
Use relative paths in packages and tools; `/workspace/` remains a supported
runtime alias. The UI shows **Files**, without adding a package directory wrapper.

Agent Files are the versioned starting files. New sessions with a new file tree
receive that starting snapshot. Attaching to an existing file tree preserves its
current files; `initial_files` on such a session request is rejected. Change
existing files through the Files API after reviewing the intended updates.
Editing or importing an agent does not rewrite existing sessions. A fork copies
current session files, and sandbox recovery uses committed session files rather
than reapplying the agent template. Durable files and live processes have separate
lifecycles; installed software and background processes depend on the Environment.

Older folders with `files/` and `skills/` still import. When `files`/`initial_files`
is omitted, legacy `files/` contents map to the working-directory root. An explicit
empty list disables that discovery. Legacy `skills/` maps to `.agents/skills/`.
If both skill roots exist, select one explicitly with `skills = [".agents/skills"]`.
New folder and ZIP exports use root-relative files and `.agents/skills/`.

## Describe MCP dependencies

Reference an MCP server already configured in your organization when the platform
owns its authentication and installation:

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
Bind this named server requirement with a same-named `AgentBuilder::mcp_server` before
applying the package. Serve explicitly resolves named environment requirements
at startup. Platform imports resolve named servers and credentials from your organization’s MCP settings.

## Validate, compare and import

```bash
# Offline; no platform credentials required.
everruns agents validate ./triage
everruns agents diff ./triage --against ./triage-before

# Check destination models, harnesses, capabilities, configured MCP servers and channels.
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

In **Agents → Import**, select a ZIP for a complete folder, or a self-contained
TOML, YAML, JSON or Markdown file. The preview follows the Agent view: rendered
instructions with a **Source** toggle, plus model/harness defaults and capabilities
in the settings column. Open **Files** for starting files, permissions and skills,
**Settings** for MCP servers and runtime settings, and **Integrations** for channels.
For an existing destination, open **Changes** to review current/imported values
and added, removed or updated files before applying. Validation failures block
importing. Export offers a complete ZIP or Markdown.

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
Validation returns `valid`, field-addressed `diagnostics`, and an authored
`preview` when parsing succeeds, even if a destination dependency is missing.
The preview contains file sizes, permissions and digests, without asset bodies
or resolved credentials. Diff requires
`target` and returns `changed` and `changes`. Export formats are `markdown`
(default), `toml`, `yaml`, `json` and `zip`.

## Sync an Agent continuously from GitHub

Open an existing Agent and select **More actions → Sync with GitHub**. Connect the
Agent's GitHub App in **Integrations** first, or choose an organization-linked
Everruns App installation there. Select an accessible repository, its branch,
and a package folder or standalone manifest. Leave the path empty for a package
at the repository root. Use **Browse folder** to navigate, then **Preview changes**
to review the same validation and changes as an import. Select **Enable continuous
sync** to apply the reviewed commit and follow subsequent changes. Built-in
Agents must be copied before connecting a source.

A repository can contain several Agents, each in its own folder:

```text
agents/
  support/
    agent.toml
    instructions.md
    .agents/skills/triage/SKILL.md
  reviewer/
    agent.toml
    instructions.md
```

Changes apply automatically after GitHub pushes. Periodic checks approximately
every minute recover missed deliveries and work without a public webhook URL.
New per-agent GitHub Apps subscribe to **Push** events. For an older App, enable
that subscription in its GitHub settings for faster updates; periodic sync still
works. The shared Everruns App needs the same subscription on its webhook.

The **Source** panel shows the applied commit, last check, last successful sync,
and errors. **Sync now** queues another check. A missing file, invalid package,
missing destination dependency or lost GitHub access retains the last successful
Agent. Reconnecting a changed GitHub connection requires selecting the source
again. If a human configured the source, that person must retain Agent management
permission in the organization.

Choose **Sync from GitHub** to keep authored Agent configuration and package-owned
channel settings read-only in Everruns, or **Sync with GitHub** to edit on either
side. Two-way sync requires **Contents: Read and write** in the GitHub App settings;
the installation must accept the updated permissions. Existing read-only bindings
stay read-only until you change the sync direction and review the source again.

In two-way mode, saved edits are queued and committed directly to the selected
branch. Branch protection still applies: a branch that requires pull requests will
reject direct updates, retain local edits and show an error. Use a branch that
permits the Agent's App to commit. The Source panel shows queued edits and conflicts.
If both sides change the package, neither version is overwritten automatically.
**Review GitHub version** previews the incoming version; enabling sync after review
replaces local authored changes. Export first if you want to save both versions,
or disconnect to keep local edits without publishing them.

Write-back preserves the selected manifest format, instruction-file location and
existing asset paths. New assets can be inline. Editing a manifest may normalize
its formatting and comments. Unrelated repository files remain untouched. Source
sync cannot change GitHub workflows. Credentials, unnamed destination channels and
channel publication remain local. Sync follows the existing Agent Files lifecycle
and never rewrites existing session files. Export and copy remain available.
**Disconnect** retains your current definition and stops new sync attempts; a
GitHub write already in flight may still complete.

The API exposes `GET`, `PUT` and `DELETE /v1/agents/{agent_id}/source`, plus
`POST .../source/options`, `POST .../source/preview` and `POST .../source/sync`.
Preview takes `repository`, `branch` and `path`; connecting supplies the returned
`commit` and `agent_digest` alongside that selection. Set `mode` to `two_way` for
write-back; omitted `mode` defaults to `pull`. Changing an existing source
also supplies its current `expected_generation`. If the reviewed source or Agent
changed, preview again before connecting.

The same operations are available under `everruns agents source --help`. Use
`preview` to review a package, `connect` with the returned commit and digest to
enable sync, `get` to inspect status, `sync` to request a check, and `disconnect`
to restore local editing. Each command includes a worked example in its help.

## Platform Chat and MCP

In Platform Chat, ask Everruns to validate, compare, import or export an agent
from the chat’s files. The assistant can run the same CLI commands:

```bash
everruns agents validate /agents/triage
everruns agents diff /agents/triage --target triage
everruns agents import /agents/triage --target triage
everruns agents export triage --format zip --out /exports/triage.zip
```

`file` and `out` refer only to the current session workspace. Reads and writes
use its existing tenant, permission and private-memory checks. Export creates a
new artifact and fails if that path already exists.

When using an MCP client connected to Everruns, these operations are available
as the tools `import_agent`, `export_agent`, `validate_agent_package` and
`diff_agent_package`. A client without a current session sends materialized
`content`; export without `out` returns the portable manifest. `--content` also accepts inline text in Platform Chat.

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

`package.create` resolves the standard `base`, `conversation`, `worker` and
`bashkit-worker` harnesses. For custom harnesses, use `Engine::create(agent).harness(...)`.

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

## Format reference and editor schema

The [agent folder and TOML reference](/reference/agent-package/) covers every
manifest field, defaults, file mappings, skill layout, MCP settings, channels,
validation limits and compatibility rules. The
[public v1 JSON Schema](/schemas/agent/v1.json) applies to the parsed TOML document.
Add its editor directive at the top of `agent.toml`:

```toml
#:schema https://everruns.com/docs/schemas/agent/v1.json
schema_version = 1
name = "my-agent"
```

Editor validation checks the document shape. Run `everruns agents validate`
for asset and skill validation, and add `--remote` to check destination bindings.

## Examples and references

- [TOML agents, root files, skills and legacy Markdown examples](https://github.com/everruns/everruns/tree/main/examples/agents)
- [Runnable Framework example](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/file_agent.rs)
- [Shared package API](https://docs.rs/everruns-core/latest/everruns_core/agent_package/)
- [REST API reference](/api/)
- [CLI](/features/cli/)
- [Framework agents](/framework/agents/) and [serve](/framework/serve/)
- [MCP configuration](/features/mcp/)

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
