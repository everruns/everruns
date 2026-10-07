---
title: Agent folders and agent.toml
description: Complete reference for portable agent directories, TOML fields, defaults, asset mappings and validation.
appliesTo: [framework, platform, cloud]
---

Use an `agent.toml` directory as the primary portable agent format. It holds
instructions, configuration and explicitly selected runtime files. The same
package loads in the Framework or serve and imports into the Platform through
the UI, HTTP API, CLI, Platform Chat and MCP. ZIP carries the complete directory.
See the [how-to guide](/how-to/define-agents-as-files/) for commands and workflows.

## Directory contract

```text
my-agent/
├── agent.toml                      # Agent configuration
├── instructions.md                 # Default instructions source
├── runbook.md                      # Selected runtime file
├── data/example.csv                # Selected runtime file
├── src/sample.py                   # Only included when selected
└── .agents/skills/investigate/
    ├── SKILL.md                    # Skill instructions and metadata
    ├── references/checklist.md      # Optional supporting reference
    └── scripts/check.py            # Optional supporting script
```

| Path | Role | Default behavior |
| --- | --- | --- |
| `agent.toml` | Manifest | Exactly one manifest at the directory root |
| `instructions.md` | Agent instructions | Read when inline `instructions` and `instructions_file` are absent |
| Paths selected by `files` | Starting session files | Keep root-relative paths; read-only by default |
| `.agents/skills/<name>/` | Bundled skill | Automatically included, with supporting assets |
| Other project files | Authoring files or unrelated code/data | Omitted unless selected |

`agent.yaml`, `agent.yml`, `agent.json` and `agent.md` are alternative manifest
encodings. Do not put multiple manifest filenames at the same root. ZIP contents
must start at the agent root, with no enclosing `my-agent/` directory.

Instructions and metadata do not become session files automatically. To make
`instructions.md` readable by a tool, explicitly select it in `files`. Runtime
files named `agent.toml`, `instructions.md` or another reserved manifest filename
are stored inline on export so authoring metadata cannot overwrite them.

Paths are relative to the working-directory root, independent of the execution
provider. There is no `workspace/` wrapper. Agent Files are a starting snapshot;
new file trees receive that snapshot once. Existing sessions and attachments to
existing trees keep their current files. A provider may use a physical directory
such as `/workspace`; that prefix is a runtime alias, not a package layout.

## Minimal manifest

```toml
#:schema https://docs.everruns.com/schemas/agent/v1.json
schema_version = 1
name = "my-agent"
```

Add `instructions.md` beside this file. Or keep a self-contained manifest:

```toml
schema_version = 1
name = "my-agent"
instructions = "Help the user with clear, accurate answers."
```

Use `instructions_file = "prompts/main.md"` for a different source path.
Nonempty inline instructions and `instructions_file` cannot be combined.

## Manifest fields

Unknown fields fail validation. TOML keys are case-sensitive. All top-level keys
must appear before the first `[table]`: a key after `[model]` belongs to the model
table, rather than to the agent.

| Field | Type | Default and rules |
| --- | --- | --- |
| `schema_version` | Integer | `1`; other versions fail. Include it in new definitions. |
| `name` | String | Required stable address: 1–255 lowercase letters, digits or hyphens, starting and ending with a letter or digit. No resource ID. |
| `display_name` | String | Optional human-readable title; host falls back to `name`. |
| `description` | String | Optional purpose or scope description. |
| `instructions` | String | Inline instructions; folders otherwise read `instructions_file` or `instructions.md`. |
| `instructions_file` | String | Safe package-relative instructions path; mutually exclusive with nonempty `instructions`. |
| `tags` | Array of strings | Empty; order does not affect diffs. |
| `model` | Table | Optional provider/model name pair; destination resolves the pair. |
| `harness` | String | Optional named harness; destination default when omitted. |
| `capabilities` | Array | Empty; stable names or `{ ref, config }` objects. Duplicate names fail. |
| `files` | Array | Explicit relative sources, globs, mappings or inline files; see below. |
| `skills` | Array of strings | Folder auto-discovers `.agents/skills/`; explicit values select directories containing direct skill subfolders. |
| `mcpServers` | Table of named servers | Empty; catalog references or inline MCP declarations. |
| `channels` | Table of named channels | Empty; transport descriptions without credentials or destination IDs. |
| `network_access` | Table | Optional `allowed`/`blocked` lists; host enforces the policy. |
| `max_iterations` | Integer | Runtime default; explicit values must be 1–1000. |
| `parallel_tool_calls` | Boolean | Runtime default; explicitly allow or disable concurrent independent calls. |
| `tools` | Array of tool tables | Empty; only client-side tool schemas, requiring executable host bindings. |
| `intro_markdown` | String | Optional Platform introduction displayed before a conversation. |
| `short_description` | String | Optional short Platform discovery text. |
| `starters` | Array of tables | Empty; Platform conversation starters with required `text` and optional `icon`. |
| `environments` | Table | Optional Platform environment declarations; explicit binding required in other hosts. |

Platform resolves its default model and harness when omitted. Framework code
binds a model explicitly. Serve uses its simulator when a model is omitted.
Framework file creation recognizes `base`, `conversation`, `worker-base` and
`worker`; custom harnesses require a host binding. Defaults that belong to the
host are not frozen into portable exports.

## Select and map files

### Preserve paths

```toml
files = ["runbook.md", "data/**"]
```

Each string selects an exact file, directory or glob relative to the package
root. `data/**` includes nested files. `data` also selects the directory's files.
Selected files keep their paths: `data/example.csv` becomes `data/example.csv`
in the session. Every declared source must match at least one file.

`files = []` deliberately includes no ordinary files. It does not disable skill
discovery. A README, source tree or neighboring agent definition is not included
unless selected. Avoid `files = ["."]` in a repository unless every allowed file
under that root should be packaged.

### Map a source to another destination

```toml
files = [
  { source = "fixtures", path = "data" },
  { source = "templates/report.md", path = "reports/current.md", is_readonly = false },
]
```

| File mapping key | Type | Default |
| --- | --- | --- |
| `source` | String | Required safe relative file, directory or glob |
| `path` | String | Omitted: preserve source paths. For a directory, prepend the destination to its contents. For a single file, use the exact destination. |
| `is_readonly` | Boolean | `true`; false lets the session edit or delete the seeded file |

A mapping of `fixtures` to `data` makes `fixtures/a.csv` land at `data/a.csv`.
A custom destination on a glob is an exact destination for every match; use a
directory mapping when preserving multiple relative child paths.

### Embed content

```toml
[[files]]
path = "notes.md"
content = "Review notes go here."
is_readonly = false

[[files]]
path = "data/sample.bin"
content = "AP8K"
encoding = "base64"
```

| Inline file key | Type | Default |
| --- | --- | --- |
| `path` | String | Required destination; prefer relative paths |
| `content` | String | Required text or Base64-encoded bytes |
| `encoding` | String | `text`; only `text` and `base64` are accepted |
| `is_readonly` | Boolean | `false` for inline files; set `true` to protect seeded content |

Source-selected files default to read-only; inline files default to writable.
Set `is_readonly` explicitly when permissions matter.

Use either the `files = [...]` form or `[[files]]` tables in one TOML document,
not both. Strings and inline mapping objects can be mixed in the array form.
Leading `/` and `/workspace/` destination aliases are accepted; exports use
relative paths. Destinations cannot traverse outside the file tree, duplicate
another destination, or collide with a parent file.

## Bundle skills

Default skill discovery reads `.agents/skills/<skill-name>/SKILL.md` and every
allowed supporting asset under that skill directory. A skill must be a direct
child folder, with valid YAML front matter and a `name` matching its directory:

```markdown
---
name: investigate
description: Investigate an issue using the bundled runbook and checklist.
---
Read runbook.md, then references/checklist.md. Report evidence and uncertainties.
```

Scripts, reference documents and binary assets retain their relative paths.
Discovery adds the `skills` capability automatically. Including a script does
not execute it; execution requires the corresponding host tool and policy.

For another source location, declare `skills = ["team-skills"]`; each child
folder lands at `.agents/skills/<name>/`. When complete skill trees are explicitly
included through `files`, they retain their declared permissions. Incomplete
skill trees and invalid or mismatched `SKILL.md` fail validation.

## Model and capabilities

```toml
capabilities = [
  "current_time",
  { ref = "session_file_system", config = {} },
]

[model]
provider = "openai"
model = "your-enabled-model"
```

`provider` and `model` must be nonempty names. Platform validation requires one
enabled matching destination model. Account availability may vary. Omit this
table to use the destination default rather than pinning a model.

A configured capability has `ref` (required string) and `config` (optional object,
default `{}`). Configuration is validated by its host capability schema. Installed
skill, MCP and plugin resource IDs are not portable capability declarations;
use bundled skills or named MCP dependencies instead.

## MCP server settings

### Use an organization MCP server

```toml
[mcpServers.issues]
use = "catalog:linear"
actsAs = "user"
```

Here **catalog** means the organization's configured MCP servers. `linear` is
the server's name at the destination; it supplies transport and authentication
settings. It is a dependency, not an exported server installation or credential.

### Declare a remote or local server

```toml
[mcpServers.docs]
type = "http"
url = "https://example.com/mcp"

[mcpServers.local]
type = "stdio"
command = "python3"
args = ["-m", "my_mcp_server"]
```

HTTP uses a safe public HTTP(S) URL without embedded credentials. Stdio requires
a nonempty command; hosted Platform imports reject local process execution.
Framework and serve can bind stdio when their MCP stdio support is enabled.

| Server key | Type | Default and rules |
| --- | --- | --- |
| `use` | String | Optional `catalog:<name>` reference; cannot be combined with inline `type`. |
| `type` | String | `http`; `http` or `stdio`. |
| `url` | String | Required for inline HTTP; ignored for stdio. |
| `command` | String | Required for stdio. |
| `args` | Array of strings | Empty; arguments for stdio. |
| `headers` | Table of strings | Empty; HTTP request headers. |
| `env` | Table of strings | Empty; stdio environment values. |
| `auth_mode` | String | `none`; `none`, `api_key` or `oauth`. Hosted credentials require destination bindings. |
| `actsAs` | String | `none`; `none`, `user`, `service` or `user_or_service` (the person's grant, else the agent's), identifying whose grant to use. |
| `deferred` | Boolean | `false`; `true` lists the server's tools only when the agent loads them through tool search. |
| `connectInChat` | String | `ask`; `ask` pauses the chat on a Connect card when a sign-in is missing, `never` fails the call with a settings link instead. |
| `oauth_provider_id` | String | Optional named provider requirement; installed MCP resource IDs are rejected. |
| `tool_discovery` | Boolean | `true`; discover server tools. |
| `protocol_mode` | String | `auto`; or pin `2025-03-26`, `2025-06-18`, `2026-07-28`. |
| `elicitation_policy` | String | `url`; `url`, `url_and_form` or `none`. |

Use `${ENV_NAME}` for credential-bearing headers/environment values:

```toml
[mcpServers.docs.headers]
Authorization = "${DOCS_AUTHORIZATION}"
```

Framework resolves placeholders through `package.bind_mcp(...)`; serve resolves
explicit named requirements at startup. Platform requires catalog bindings for
credentials and never reads arbitrary worker environment variables on import.

## Channel descriptions

```toml
[channels.chat]
type = "ag_ui"
enabled = false
[channels.chat.config]
tool_visibility = "generic"
generic_tool_text = "Working…"
rate_limit_per_minute = 30
```

| Channel key | Type | Default and rules |
| --- | --- | --- |
| Table name | Stable name | Lookup/upsert name at the destination; no channel ID. |
| `type` | String | Required known transport: `ag_ui`, `public_chat`, `slack`, `fcp`, `a2a`, `api_endpoint`, `schedule`, `webhook`. |
| `enabled` | Boolean | `false`; true requests an enabled draft, not automatic publication. |
| `config` | Table | `{}`; transport-specific declarative settings without credentials or resource IDs. |

Platform imports create `ag_ui`, `public_chat`, `fcp` and `slack` channels.
Public Chat also requires its feature flag. Other known types require their
host/dedicated APIs; Platform validation rejects unsupported bindings. Schedules
remain trigger resources. Framework and serve bind transports separately.
See [Channels](/features/channels/) for transport-specific settings.

Updates upsert declared channels, preserve omitted channels and existing
activation/credentials, and require permission to modify live configuration.
Packages do not transfer publication, grants, sessions, history or deployment state.

## Optional policies and Platform presentation

```toml
intro_markdown = "Welcome to the incident desk."
short_description = "Investigate incidents with evidence."
starters = [{ text = "Investigate the current incident.", icon = "search" }]

[network_access]
allowed = ["https://docs.everruns.com/**"]
blocked = ["https://example.com/private/**"]
```

`blocked` takes precedence. An empty `allowed` list imposes no allowlist
restriction. Host policy can further restrict access. Network configuration
is descriptive until an enforcing host binds it.

`sandbox_policy` retains the Platform's Sandbox policy; see
[Sandbox Templates](/features/sandbox-templates/) for the host configuration.
The generic Framework package builder rejects this policy until explicitly bound.

A `tools` declaration contains a `client_side` type, `name`, `description` and
object JSON Schema `parameters`. Optional display metadata and execution hints
follow the [tool schema in the public agent schema](/schemas/agent/v1.json).
The Framework requires a matching host handler and schema; declarations do not
transfer executable closures. Built-in tools are selected through capabilities.

## Validation and limits

| Limit | Value |
| --- | --- |
| Package bytes | 10 MiB |
| Runtime files | 100 |
| Each asset / instructions | 1 MiB |
| Total decoded runtime file content | 5 MiB |
| Channel descriptions | 32 |
| Native folder nesting / visited entries | 32 levels / 1,024 entries |

Validation rejects unknown fields, unsupported versions, unsafe source paths,
traversal, symlinks, duplicate or parent-conflicting destinations, malformed
Base64, incomplete skills, literal credentials and nonportable resource IDs.
Folder/glob collection skips disallowed hidden paths, including `.env`, `.ssh`
and `.git`; explicit credential-file sources fail. Explicit inline content is
already authored data and is not a host-file read. Review it before sharing.

Editor schema validation cannot inspect file bytes, symlinks, decoded sizes,
credential policy or destination dependencies. Run:

```bash
everruns agents validate ./my-agent
everruns agents validate ./my-agent --remote
```

## Public schema and editor integration

Download the [v1 agent schema](/schemas/agent/v1.json). It is a standalone JSON
Schema for the parsed TOML document, with named definitions for file forms,
capabilities, MCP settings and channels. YAML and JSON use the same shape.
It is generated from the public manifest types, with portable validation bounds.

Use this header for completion and diagnostics in editors supporting the
[Taplo schema directive](https://taplo.tamasfe.dev/configuration/directives.html):

```toml
#:schema https://docs.everruns.com/schemas/agent/v1.json
```

This is a comment, not an agent field. Do not add a `$schema` field to the
manifest: unknown keys are rejected. Offline editors can download the schema
and use `#:schema ./agent-v1.schema.json` instead.

## Compatibility

Legacy Markdown with YAML front matter still imports; its body becomes
instructions. `system_prompt`, `initial_files`, `harness_name` and `mcp_servers`
remain input aliases. Unversioned legacy files may contain platform IDs; new
versioned definitions and exports omit them. The public schema describes
canonical authoring, rather than every legacy input alias.

Old `files/` contents map to the runtime root when `files`/`initial_files` is
omitted. Explicit `files = []` disables that discovery. Old `skills/` maps to
`.agents/skills/`. If both skill roots exist, explicitly choose one using
`skills = [".agents/skills"]`. New exports use root-relative files.
