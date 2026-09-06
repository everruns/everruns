---
type: Specification
title: "Skills Registry Specification"
description: "Agent Skills registry."
tags:
  - everruns
  - project
---
# Skills Registry Specification

## Abstract

This document defines the skills registry for Everruns, a system for storing, discovering, validating, and serving [Agent Skills](https://agentskills.io/) to agents. Skills are portable instruction packages (SKILL.md files with optional scripts, references, and assets) that extend agent capabilities with specialized knowledge and workflows. The registry follows the [Agent Skills open specification](https://agentskills.io/specification).

Skills integrate into the existing capability system as "virtual capabilities" (similar to MCP servers), allowing agents to discover and activate skills on demand.

## Background & Prior Art

### Agent Skills Format (agentskills.io)

The Agent Skills format is an open standard originally developed by Anthropic. A skill is a directory containing:

```
skill-name/
├── SKILL.md          # Required: YAML frontmatter + markdown instructions
├── scripts/          # Optional: executable code
├── references/       # Optional: additional documentation
└── assets/           # Optional: templates, data files
```

**SKILL.md frontmatter** (required fields):
```yaml
---
name: skill-name          # 1-64 chars, lowercase alphanumeric + hyphens
description: What this skill does and when to use it.  # 1-1024 chars
---
```

Optional frontmatter: `license`, `compatibility`, `metadata` (key-value map), `allowed-tools` (experimental), `user-invocable` (Everruns command visibility extension; see [`knowledge/project/commands.md`](commands.md)), `disable-model-invocation` (prevents the model from auto-invoking the skill; see below).

#### Invocation Control Fields

Two independent boolean frontmatter fields control who can invoke a skill:

| Frontmatter | User can invoke (/) | Model can invoke | In system prompt |
|---|---|---|---|
| *(defaults)* | Yes | Yes | Description listed |
| `disable-model-invocation: true` | Yes | No | Description **not** listed |
| `user-invocable: false` | No | Yes | Description listed |
| Both set | No | No | **Unreachable** (validation warning) |

- `user-invocable: false`, hides from `/` autocomplete menu (background knowledge only)
- `disable-model-invocation: true`, prevents the model from seeing the skill in its system prompt, so it cannot auto-invoke it. The skill is still invocable via explicit `/name` slash command.

**Progressive disclosure** is core to the design:
1. **Discovery** (~100 tokens): Only name + description loaded at startup
2. **Activation** (<5000 tokens recommended): Full SKILL.md body loaded when matched
3. **Resources** (as needed): Scripts/references/assets loaded on demand

### How Other Platforms Handle Skill/Plugin Registries

| Platform | Storage | Upload Format | Discovery | Validation |
|----------|---------|---------------|-----------|------------|
| npm | Central registry (npmjs.com) | tar.gz packages | `npm search`, web UI | package.json schema, semver |
| Docker Hub | Central registry | OCI images (layers) | `docker search`, web UI | Dockerfile linting, manifest validation |
| VS Code Marketplace | Central registry | .vsix (zip) | Marketplace web, CLI | manifest.json schema, publisher verification |
| Agent Skills | Local `.agents/skills/` dirs | Directories with SKILL.md | Filesystem scan | SKILL.md frontmatter parsing |
| MCP Servers (Everruns) | PostgreSQL | API (URL + config) | API listing, capability system | Name/URL validation, tool caching |

**Key insight**: Most registries combine a metadata store (for discovery) with a content store (for the actual payload). For Everruns, PostgreSQL handles metadata and content in a single system, matching the MCP server pattern.

## Requirements

### Model

A registered skill carries the parsed SKILL.md frontmatter plus its body, the org that owns it, a
status, and, for archive uploads, the original ZIP alongside its extracted files. Archive files are
stored individually rather than unpacked on demand, so activation and VFS mounting never pay for ZIP
extraction at runtime.

Fields and types: [`Skill` and `SkillFileEntry`](../../crates/core/src/skill.rs). Persistence:
`skills` and `skill_files` in [`crates/server/migrations/001_base_schema.sql`](../../crates/server/migrations/001_base_schema.sql).

Two source types exist: `markdown` (a pasted SKILL.md, instructions only) and `archive` (a ZIP
holding the full skill directory with scripts, references, and assets). Statuses come from
[`SkillStatus`](../../crates/core/src/skill.rs); only `active` skills are offered to agents.

### Limits and validation

Names follow the agentskills.io rules exactly, 1–64 characters, `[a-z0-9-]`, no leading, trailing,
or consecutive hyphens, and are unique per organization, which keeps capability IDs unambiguous
across tenants.

Every other bound (field lengths, archive size, file count, decompressed size) exists to keep skills
what they are: instructions, not binary packages. The enforced numbers live in
[`crates/server/src/domains/skills/archive.rs`](../../crates/server/src/domains/skills/archive.rs) and
the SKILL.md parser in [`crates/core/src/skill.rs`](../../crates/core/src/skill.rs).

### API surface

Routes and their request/response types live in
[`crates/server/src/api/skills.rs`](../../crates/server/src/api/skills.rs) and are published in the
OpenAPI export; that is the contract, not this list. What the endpoints must preserve:

- **Two creation paths.** JSON `skill_md` for a pasted SKILL.md (`source_type = markdown`), and a
  multipart ZIP upload for a full skill directory (`source_type = archive`). Both parse and validate
  the frontmatter before anything is stored, and reject duplicate names within the org.
- **Archive intake is validated at the boundary**: SKILL.md located in the archive root or in a
  single top-level directory, frontmatter name matching the directory name, and the structural
  limits above enforced before extraction. The original ZIP is retained for re-download while the
  extracted files land in `skill_files` for VFS mounting.
- **Content retrieval is activation's data path.** Fetching a skill's content returns the SKILL.md
  body plus its bundled files, so activation is a read rather than a runtime unpack.
- **Validation without side effects.** A skill can be checked before it is created, returning
  structured errors and warnings, so clients can validate SKILL.md as the user types.

## Skills as Virtual Capabilities

Skills integrate into the capability system as virtual capabilities, following the MCP server pattern.

### Capability ID Format

```
skill:{skill_uuid}
```

Example: `skill:01933b5a-0000-7000-8000-000000000601`

### Progressive Disclosure in Capability System

1. **Listing** (GET /v1/capabilities): Returns skill name + description only (metadata for discovery)
2. **Agent Assignment**: Skill capability ID stored in `agent_capabilities` junction table
3. **Session Runtime**: When an agent with skill capabilities starts a session:
   - Skill names and descriptions injected into system prompt as `<available_skills>` XML block
   - Agent can "activate" a skill by requesting its full instructions
4. **Activation**: Full SKILL.md body loaded into context when agent matches a task to a skill

### System Prompt Integration

When skills are assigned to an agent, the system prompt includes:

```xml
<available_skills>
  <skill>
    <name>pdf-processing</name>
    <description>Extract text and tables from PDF files, fill forms, merge documents.</description>
  </skill>
  <skill>
    <name>data-analysis</name>
    <description>Analyze datasets, generate charts, and create summary reports.</description>
  </skill>
</available_skills>

When a user's task matches an available skill, activate it by using the `activate_skill` tool with the skill name. This loads the full instructions into your context.
```

### Skill Activation Tool

Skills provide a virtual tool for activation:

| Tool | Parameters | Description |
|------|-----------|-------------|
| `activate_skill` | `{ "name": "skill-name" }` | Loads the full SKILL.md instructions into the agent's context |

Activation resolves the skill by name and returns its full SKILL.md body as the tool result, which
the agent then follows. For archive-based skills, the instructions reference companion files by
relative path and the agent reads them with ordinary filesystem tools. Because registry skills are
already mounted into the session VFS before the tool runs, activation is a read, not a fetch.

#### Idempotence

`activate_skill` is idempotent within a session. The first successful activation is recorded in the session resource registry under `resource_id = "skill_activation:{name}"` with the tool result stored as metadata. Subsequent calls for the same skill short-circuit and return the cached result with an extra `already_active: true` flag, no VFS re-read, no re-parse, no re-mount. This keeps the handle contract stable across retries and planner loops that re-emit the same activation.

Fallback: when the runtime does not wire a session resource registry into the tool context (embedded callers, unit tests), `activate_skill` skips the cache and re-executes normally. The user-visible semantics are unchanged; only the "already active" signal is suppressed.

### Bundled File Access via VFS Mounting

For archive-based skills, extracted files are mounted into the session's virtual filesystem when the skill is activated. This reuses the existing `MountPoint` / `MountSource` system that capabilities already use.

**Mount path**: `/.agents/skills/{skill-name}/`

Example: A skill named `pdf-processing` with files `scripts/extract.py` and `references/REFERENCE.md` would be mounted as:
```
/.agents/skills/pdf-processing/
├── SKILL.md
├── scripts/extract.py
└── references/REFERENCE.md
```

The agent can then read these files using existing session filesystem tools (`read_file`), no special `read_skill_file` tool needed.

**Mounting strategy**: registry skills become read-only `MountPoint`s carrying each file inline,
text or base64 for binaries, built by
[`AttachSkillCapability`](../../crates/builtins/src/attach_skill.rs) during capability
collection, before any tool runs. The `activate_skill` result carries instructions and metadata
(`skill`, `description`, fork-mode fields where applicable) and deliberately no companion-file
listing.

**Non-active references degrade, they do not fail**: a `skill:{uuid}` ref is accepted at any status
by capability validation, so a referenced skill that is archived, disabled, or deleted is skipped
with a warning at mount collection and the session still starts. Archiving a skill must not take
down every agent that references it; the agent simply does not see that skill.

This approach:
- Reuses existing VFS infrastructure (no new tool needed)
- Files are accessible via the same `read_file` / `list_files` tools the agent already has
- Consistent with how other capabilities mount content (e.g., `sample_data`)

**No `bundled_files` in tool result**: The `activate_skill` tool result does not include a separate list of companion file paths. The agent discovers referenced files from relative paths in the SKILL.md instructions (per the [agentskills.io progressive disclosure model](https://agentskills.io/specification)), or via `list_files` on the skill directory. A well-written SKILL.md already references every file the agent needs.

### Capability-Contributed Skills

Beyond user-uploaded (registry) and filesystem skills, any `Capability` can ship skills in code via `contribute_skills() -> Vec<SkillContribution>`. See `knowledge/execution/capabilities.md` for the trait method and `crates/core/src/capabilities/skill_contribution.rs` for the neutral `SkillContribution` values.

Contributed skills flow through the **same** discovery/activation path as other skills:

1. During capability collection, each `SkillContribution` is normalized into a read-only `MountPoint` at `/.agents/skills/{name}/` with a reconstructed `SKILL.md` (frontmatter + body) and every bundled file.
2. When the built-in `skills` capability is active, its VFS scan finds these mounts alongside `AttachSkillCapability` mounts and filesystem-resident skills.
3. `list_skills`, `activate_skill`, prompt listing, and `/slash` command visibility all go through the existing path, the frontmatter flags `user-invocable` and `disable-model-invocation` are honored the same way.

Reconstruction preserves description text exactly, including YAML-sensitive quotes, backslashes and line breaks, so discovery and activation see the same metadata supplied by the contributor.

The mount's `capability_id` is set to the contributing capability's ID so the VFS layer attributes the mounted files correctly and so users can see which capability a skill came from. There is no separate database row and no parallel prompt-injection path: if a capability wants a skill, it returns a `SkillContribution` and the rest is shared pipeline.

## Database Schema

`skills` and `skill_files` are defined in
[`crates/server/migrations/001_base_schema.sql`](../../crates/server/migrations/001_base_schema.sql).
Two shapes carry design intent worth stating here: skills are unique per `(org_id, name)` so
capability IDs cannot collide across tenants, and archive files are rows rather than a blob to unpack,
because every activation mounts them into the session VFS.

## Activation Substitution Pipeline

When `activate_skill` runs, the SKILL.md body is transformed through a fixed pipeline before being returned to the model. All steps are applied to the body only, frontmatter is parsed separately and never substituted.

1. **Argument expansion** (`$ARGUMENTS`, `$ARGUMENTS[N]`, `$N`), synchronous, always runs.
2. **Environment substitution** (`${SESSION_ID}`, `${SKILL_DIR}`), synchronous, always runs.
3. **Command injection** (`` !`cmd` ``), asynchronous shell execution inside the session sandbox (bashkit shell / VFS). Runs ONLY for trusted sources. The dormant default executor still targets the worker host; see the trust-gate section below.

### Command-Injection Trust Gate

``!`cmd` `` placeholders let a skill inline the stdout of a shell command at activation time (e.g. ``!`git rev-parse --show-toplevel` ``, ``!`date` ``). Because this spawns a shell process on the worker host, it is only safe for SKILL.md content that came from a non-user-spoofable source.

**Current status: the gate is forced off.** `SessionFile::is_readonly` is **not** a valid trust signal, both the session-files HTTP API (create/update) and `InitialFile` configuration accept `is_readonly = true` from user input, so a user could mark a SKILL.md readonly and regain RCE.

| Source | Mount mode | Trust gate outcome |
|---|---|---|
| Capability-contributed (`contribute_skills`) | `MountPoint::readonly` → `is_readonly = true` on DB row | **UNTRUSTED today**: ``!`cmd` `` stays literal |
| Registry-attached (`AttachSkillCapability`) | `MountPoint::readonly` → `is_readonly = true` on DB row | **UNTRUSTED today**: ``!`cmd` `` stays literal |
| User-uploaded via VFS write (e.g. agent `write_file` at runtime) | writable → `is_readonly = false` | **UNTRUSTED**: ``!`cmd` `` stays literal |
| Agent / session `initial_files` (any `is_readonly` value) | readonly or writable | **UNTRUSTED**: ``!`cmd` `` stays literal |
| Session-files API create/update with `is_readonly = true` | readonly | **UNTRUSTED**: ``!`cmd` `` stays literal |

Re-enabling the feature requires BOTH:

1. A platform-controlled provenance signal (for example, a `mount_capability_id` column on `session_files` that is populated only by mount application code and rejected on all user-facing API paths), AND
2. Replacing the default `ProcessCommandExecutor` (which spawns worker-host `bash -c`) with a session-sandbox-backed executor so commands run against **the bashkit shell (managed session sandbox) and the session virtual filesystem**, not the worker host. Flipping provenance alone is insufficient, a trusted but misbehaving skill would otherwise be able to reach worker state.

See threat-model entry [`TM-TOOL-020`](../security/threat-model.md) for the mitigation state and EVE-388 for follow-up.

Enforcement lives at a single call site in `ActivateSkillFromVfsTool::execute_with_context` (`crates/builtins/src/skills.rs`). The `preprocess_command_injections` function in `crates/core/src/skill.rs` is kept wired up (with unit tests) so the re-enable follow-up only needs to flip the gate after introducing the provenance field. The function is bounded (`MAX_COMMAND_PLACEHOLDERS_PER_SKILL` = 32 placeholders per activation, concurrency cap of 4 shells) so a trusted-but-large SKILL.md cannot exhaust worker resources.

## Security Considerations

1. **Archive intake is hostile input.** ZIP handling must reject path traversal and bound file count,
   individual file size, and total decompressed size so an upload cannot become a zip bomb. The
   enforced bounds live in [`archive.rs`](../../crates/server/src/domains/skills/archive.rs).
2. **Script execution stays sandboxed.** Skills may ship scripts; they run through `bashkit_shell`
   under existing session filesystem permissions, and are logged for auditing.
3. **Content sanitization is a UI concern.** SKILL.md reaches agents as markdown, not a browser, so
   the agent path needs no HTML sanitization, but anything the UI renders does.
4. **Per-organization name uniqueness** prevents capability ID conflicts across tenants.

## Implementation Details

### Crate Structure

| Crate | Responsibility |
|-------|----------------|
| `everruns-core` | Skill types, SKILL.md parser, name validation, stable capability identity and contribution values |
| `everruns-builtins` | `AttachSkillCapability` + `SkillsCapability` implementations |
| `everruns-server` | API routes, gRPC services, database operations, ZIP handling |
| `everruns-worker` | No skill-specific role, the `activate_skill` / `list_skills` tools execute in-process from `everruns-builtins` (`SkillsCapability`) |

### Key Components

| Concern | Source |
|---|---|
| SKILL.md parsing, name validation, `Skill` types | [`crates/core/src/skill.rs`](../../crates/core/src/skill.rs) |
| `skills` capability: VFS scan, `list_skills`, `activate_skill` | [`crates/builtins/src/skills.rs`](../../crates/builtins/src/skills.rs) |
| `skill:{uuid}` mount-only capability for registry skills | [`crates/builtins/src/attach_skill.rs`](../../crates/builtins/src/attach_skill.rs) |
| CRUD, archive extraction, capability listing | [`crates/server/src/domains/skills/`](../../crates/server/src/domains/skills) |

The division that matters: `AttachSkillCapability` only mounts, it contributes no prompt text and no
tools, while `SkillsCapability` owns discovery and activation for every source. Activation is
therefore a VFS read, with no worker-side executor and no gRPC fetch in the path.

### Error Handling

| Error | Response |
|-------|----------|
| Invalid SKILL.md frontmatter | `422 Unprocessable Entity` with validation errors |
| Duplicate skill name | `409 Conflict` |
| Skill not found | `404 Not Found` |
| Invalid ZIP archive | `422 Unprocessable Entity` |
| Archive too large | `413 Payload Too Large` |
| Archive path traversal detected | `422 Unprocessable Entity` |

## UI Integration

### Skills Page

Top-level route at `/skills` in the main sidebar (same level as Agents, Sessions, Capabilities). Components:

1. **Skills List**: Cards showing each skill with name, description, status badge, source type badge
2. **Add Skill Dialog**: Two tabs:
   - **Paste SKILL.md**: Text area for pasting SKILL.md content
   - **Upload ZIP**: File upload for ZIP archives
3. **Skill Detail View**: Shows full metadata, instructions preview, bundled files list (for archives)
4. **Validation Feedback**: Real-time validation of SKILL.md content as user types

### Capability Selector

In the agent capability selector, skills appear with:
- A "Skill" badge (distinguishing from built-in and MCP capabilities)
- Skill name as capability name
- Skill description as capability description
- Category: "Skills"

## Seed Data

No skills are seeded. The registry starts empty; `examples/skills/` carries the reference examples.

## Filesystem Discovery

Skills can also be discovered from the session filesystem at `/.agents/skills/`. Each subdirectory containing a `SKILL.md` file is treated as a skill.

### Discovery Path

```
/.agents/skills/
├── hello-world/
│   └── SKILL.md
├── csv-analyzer/
│   ├── SKILL.md
│   ├── scripts/analyze.py
│   └── references/REFERENCE.md
```

### Discovery Flow

1. On session startup (when `skills` capability is enabled), scan `/.agents/skills/` for subdirectories
2. For each directory containing a `SKILL.md`, parse the frontmatter
3. Valid skills are registered as available skills in the `SkillsCapability`
4. Invalid `SKILL.md` files are logged as warnings and skipped
5. Discovered skills appear in the `<available_skills>` system prompt block alongside registry-based skills

### Discovery vs Registry

| Feature | Registry (API) | Filesystem (`.agents/skills/`) |
|---------|---------------|-------------------------------|
| Storage | PostgreSQL | Session VFS |
| Persistence | Org-wide, cross-session | Per-session |
| Upload | API endpoints | Write files to VFS |
| Capability ID | `skill:{uuid}` | `skills` (aggregate) |
| Best for | Shared/reusable skills | Project-specific skills |

A third source, **capability-contributed skills**: also feeds this pipeline. Any `Capability` can ship skills in code via `contribute_skills()` (see `knowledge/execution/capabilities.md`); those contributions mount at the same `/.agents/skills/{name}/` path and are served through the same `skills` capability. They are per-session (scoped to the contributing capability's activation) and best for bundling a reusable workflow with the capability that powers it, e.g., a `gpt_image_gen` capability shipping a "prompt an image" skill alongside its tools.

### Multi-Scope Discovery (`ScopedSkillsCapability`)

The default `SkillsCapability` scans the single `/.agents/skills/` root and exposes
`list_skills` + `activate_skill`. Embedders that need **multiple labeled skill
sources**: for example a terminal coding agent with workspace, per-user *global*,
and binary-bundled *system* scopes, can register `ScopedSkillsCapability` instead.
It takes a `SkillsConfig`:

- **`scopes`**: an ordered list of `SkillScope { label, vfs_root, writable }`, highest
  precedence first. Discovery merges all scopes and de-duplicates by skill directory
  name, so a nearer scope shadows a farther one. Each `list_skills` entry is tagged
  with its scope.
- **`resolver`**: a `SkillDirResolver` that produces the `${SKILL_DIR}` value and the
  agent-facing display path. The default keeps both in the VFS namespace; an embedder
  whose shell runs in a different namespace (e.g. a CLI whose `bash` runs on the host)
  overrides it to return a path valid there. This is the boundary that lets `${SKILL_DIR}`
  and the discovery file store stay namespace-consistent.
- **`manage_tools`**: when set, additionally exposes `read_skill` and `write_skill`.
  `write_skill` installs/updates a skill in a **writable** scope (system/bundled scopes
  are read-only), validating the name, matching the frontmatter `name`, bounding extra
  files, and rejecting path traversal and `SKILL.md` overrides.

**Sources are VFS-bound by construction.** A scope is a *labeled VFS root*, never a host
filesystem path, and every discovery/read/write goes through the injected
`SessionFileSystem`. The capability therefore cannot be pointed outside the session VFS:
how each VFS root maps to storage (a sandboxed overlay on the server, a real on-disk
directory in a single-user CLI) is decided by the file store, not by any configuration
knob on the capability. There is deliberately no API that accepts a host path. The
command-injection trust gate is unchanged, `` !`cmd` `` is never expanded
(EVE-388 / TM-TOOL-020).

## Example Skills

Example skills are provided in `examples/skills/`:
- `hello-world/`, Minimal skill demonstrating the SKILL.md format
- `csv-analyzer/`, Complex skill with scripts and references (archive-based)

## Design Decisions

| Question | Decision | Rationale |
|----------|----------|-----------|
| Why store content in DB? | PostgreSQL BYTEA/TEXT | Consistent with image storage pattern. No external object store needed for MVP. |
| Why not filesystem storage? | DB is simpler | Everruns is a managed service. Filesystem introduces deployment complexity. |
| Why both markdown and archive upload? | Flexibility | Simple skills are just text. Complex skills need bundled scripts/assets. |
| Why virtual capabilities? | Reuse existing system | MCP servers proved this pattern. Same capability selector, same agent assignment. |
| Why progressive disclosure? | Token efficiency | Loading all skill instructions at startup wastes context. Load on demand. |
| Why `activate_skill` tool? | Agent-driven activation | Agent decides when a skill is relevant, matching the agentskills.io design. |
| Why per-org uniqueness? | Multitenancy | Different orgs can have skills with same name. |
| Why 10 MB archive limit? | Practical bound | Skills are instructions, not large binary packages. |
| Why keep original ZIP + extracted files? | Both needed | ZIP for reference/re-download. Extracted rows for fast reads and VFS mounting (no runtime extraction). |
| Why no `bundled_files` in activate result? | Redundant | SKILL.md already references companion files via relative paths (agentskills.io spec). Agent can also use `list_files`. Listing paths wastes tokens. |
| Why `skill_files` table? | VFS mounting | Session VFS needs individual file content. Extracting from ZIP on every activation is wasteful. |
| Why VFS mounting instead of `read_skill_file` tool? | Reuse existing infra | Session filesystem tools (`read_file`, `list_files`) already exist. No new tool needed. Consistent with how `sample_data` capability mounts files. |
