# Skills Registry Specification

## Abstract

This document defines the skills registry for Everruns — a system for storing, discovering, validating, and serving [Agent Skills](https://agentskills.io/) to agents. Skills are portable instruction packages (SKILL.md files with optional scripts, references, and assets) that extend agent capabilities with specialized knowledge and workflows. The registry follows the [Agent Skills open specification](https://agentskills.io/specification).

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

Optional frontmatter: `license`, `compatibility`, `metadata` (key-value map), `allowed-tools` (experimental).

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
| Claude Code Skills | Local `.claude/skills/` dirs | Directories with SKILL.md | Filesystem scan | SKILL.md frontmatter parsing |
| MCP Servers (Everruns) | PostgreSQL | API (URL + config) | API listing, capability system | Name/URL validation, tool caching |

**Key insight**: Most registries combine a metadata store (for discovery) with a content store (for the actual payload). For Everruns, PostgreSQL handles metadata and content in a single system, matching the MCP server pattern.

## Requirements

### Skill

A registered skill in the Everruns registry.

| Field | Type | Description |
|-------|------|-------------|
| `id` | UUID v7 | Internal identifier |
| `public_id` | SkillId | External identifier (`skill_{32-hex}`) |
| `org_id` | i64 | Owning organization |
| `name` | string | Unique skill name (from SKILL.md frontmatter) |
| `description` | string | Skill description (from SKILL.md frontmatter) |
| `license` | string? | License info (from frontmatter) |
| `compatibility` | string? | Environment requirements (from frontmatter) |
| `metadata` | json | Arbitrary key-value metadata (from frontmatter) |
| `allowed_tools` | string? | Pre-approved tools list (from frontmatter, experimental) |
| `instructions` | text | Full SKILL.md markdown body (after frontmatter) |
| `source_type` | enum | `markdown` or `archive` |
| `archive_data` | bytes? | ZIP archive contents (when source_type = archive) |
| `status` | enum | `active` or `disabled` |
| `version` | string | Version from metadata or "1.0" default |
| `created_at` | timestamp | Creation time |
| `updated_at` | timestamp | Last modification time |

**Input Validation Limits:**

| Field | Max Size | Notes |
|-------|----------|-------|
| `name` | 64 chars | Must match agentskills.io spec: lowercase alphanumeric + hyphens, no leading/trailing/consecutive hyphens |
| `description` | 1024 chars | Non-empty, from frontmatter |
| `license` | 500 chars | Optional |
| `compatibility` | 500 chars | Optional |
| `instructions` | 100 KB | Markdown body (recommended <500 lines) |
| `archive_data` | 10 MB | ZIP archive with skill directory |
| `metadata` | 10 KB | JSON object |
| `allowed_tools` | 1 KB | Space-delimited tool list |

### Source Types

| Type | Description |
|------|-------------|
| `markdown` | Single SKILL.md file upload (instructions only, no scripts/assets) |
| `archive` | ZIP archive containing full skill directory (SKILL.md + optional scripts/, references/, assets/) |

### Status Values

| Status | Description |
|--------|-------------|
| `active` | Skill is available for agent use |
| `disabled` | Skill is disabled and hidden from capability listing |

### Name Validation Rules

Following the agentskills.io specification exactly:

1. Must be 1-64 characters
2. Lowercase letters, numbers, and hyphens only (`[a-z0-9-]`)
3. Must not start or end with `-`
4. Must not contain consecutive hyphens (`--`)
5. Must be unique per organization

### API Endpoints

#### POST /v1/skills

Create a new skill from a SKILL.md file (markdown body with frontmatter).

**Request Body (JSON):**
```json
{
  "skill_md": "---\nname: pdf-processing\ndescription: Extract text from PDFs.\n---\n\n# PDF Processing\n\n## Steps\n1. Use pdfplumber..."
}
```

**Response:** `201 Created` with Skill object

**Validation:**
- Parses YAML frontmatter from `skill_md`
- Validates `name` field against naming rules
- Validates `description` is non-empty and within limits
- Checks for duplicate name within org
- Sets `source_type = "markdown"`

#### POST /v1/skills/upload

Create a skill from a ZIP archive containing a skill directory.

**Request:** `multipart/form-data` with `file` field containing ZIP archive

**Response:** `201 Created` with Skill object

**Validation:**
- Extracts ZIP archive
- Locates SKILL.md in archive root (or single top-level directory)
- Parses and validates SKILL.md frontmatter
- Validates archive structure (no path traversal, size limits)
- Validates `name` matches directory name (if present)
- Stores both parsed metadata and original archive
- Sets `source_type = "archive"`

#### GET /v1/skills

List all skills.

**Response:** `200 OK`
```json
{
  "data": [Skill, ...]
}
```

#### GET /v1/skills/{skill_id}

Get a specific skill by ID.

**Response:** `200 OK` with Skill object, or `404 Not Found`

#### GET /v1/skills/{skill_id}/content

Get the full SKILL.md content for a skill (used during skill activation).

**Response:** `200 OK`
```json
{
  "skill_md": "---\nname: ...\n---\n\n# Instructions...",
  "files": [
    {
      "path": "scripts/extract.py",
      "content": "#!/usr/bin/env python3\n..."
    },
    {
      "path": "references/REFERENCE.md",
      "content": "# Detailed Reference\n..."
    }
  ]
}
```

For `source_type = "markdown"`: Returns `skill_md` only, `files` is empty.
For `source_type = "archive"`: Returns `skill_md` and extracted file listing.

#### PATCH /v1/skills/{skill_id}

Update a skill. Only provided fields are updated.

**Request Body:**
```json
{
  "skill_md": "---\nname: pdf-processing\ndescription: Updated description.\n---\n\nUpdated instructions...",
  "status": "disabled"
}
```

**Response:** `200 OK` with updated Skill object

#### DELETE /v1/skills/{skill_id}

Delete a skill.

**Response:** `204 No Content` on success, `404 Not Found` if not exists

#### POST /v1/skills/validate

Validate a SKILL.md without creating a skill. Useful for client-side validation.

**Request Body:**
```json
{
  "skill_md": "---\nname: my-skill\ndescription: Does something.\n---\n\nInstructions..."
}
```

**Response:** `200 OK`
```json
{
  "valid": true,
  "name": "my-skill",
  "description": "Does something.",
  "warnings": ["Instructions exceed 500 lines (recommended max). Consider splitting into references."]
}
```

Or:
```json
{
  "valid": false,
  "errors": ["name: consecutive hyphens not allowed", "description: must not be empty"]
}
```

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

When the agent calls `activate_skill`:
1. Worker resolves skill by name from the registry (via gRPC)
2. Full SKILL.md body returned as tool result
3. Agent uses the loaded instructions to perform the task
4. For archive-based skills, file paths in instructions can be resolved via additional tool calls

### Bundled File Access

For archive-based skills, an additional tool enables access to bundled files:

| Tool | Parameters | Description |
|------|-----------|-------------|
| `read_skill_file` | `{ "name": "skill-name", "path": "scripts/extract.py" }` | Reads a bundled file from the skill archive |

## Database Schema

```sql
CREATE TABLE skills (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    public_id TEXT NOT NULL,
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) DEFAULT 1,
    name VARCHAR(64) NOT NULL,
    description VARCHAR(1024) NOT NULL,
    license TEXT,
    compatibility VARCHAR(500),
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    allowed_tools TEXT,
    instructions TEXT NOT NULL,
    source_type VARCHAR(20) NOT NULL DEFAULT 'markdown'
        CHECK (source_type IN ('markdown', 'archive')),
    archive_data BYTEA,
    status VARCHAR(50) NOT NULL DEFAULT 'active'
        CHECK (status IN ('active', 'disabled')),
    version VARCHAR(50) NOT NULL DEFAULT '1.0',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT skills_public_id_format CHECK (public_id ~ '^skill_[0-9a-f]{32}$')
);

CREATE UNIQUE INDEX idx_skills_org_public_id ON skills(org_id, public_id);
CREATE UNIQUE INDEX idx_skills_org_name ON skills(org_id, name);
CREATE INDEX idx_skills_status ON skills(status);
CREATE INDEX idx_skills_org_id ON skills(org_id);
```

## Security Considerations

1. **Archive Validation**: ZIP archives must be validated for:
   - Path traversal attacks (no `../` in file paths)
   - Zip bombs (decompressed size limits)
   - File count limits (max 100 files)
   - Individual file size limits (1 MB per file)
   - Total decompressed size limit (10 MB)

2. **Script Execution**: Skills may contain scripts. Execution should be:
   - Sandboxed (via virtual_bash capability)
   - Logged for auditing
   - Subject to existing session filesystem permissions

3. **Content Sanitization**: SKILL.md content is rendered as markdown to agents, not to browsers. No HTML sanitization needed for the agent path, but the UI should sanitize any skill content rendered in the browser.

4. **Name Uniqueness**: Per-organization uniqueness prevents capability ID conflicts.

## Implementation Details

### Crate Structure

| Crate | Responsibility |
|-------|----------------|
| `everruns-core` | Skill types, SKILL.md parser, name validation, `SkillCapability` impl |
| `everruns-server` | API routes, gRPC services, database operations, ZIP handling |
| `everruns-worker` | `SkillToolExecutor` for `activate_skill` and `read_skill_file` tool execution |

### Key Components

**SkillMdParser** (`crates/core/src/skill.rs`):
- Parses YAML frontmatter from SKILL.md content
- Validates name, description, and optional fields
- Returns structured `ParsedSkillMd` with metadata and body

**SkillCapability** (`crates/core/src/capabilities/skill.rs`):
- Implements `Capability` trait
- Returns `activate_skill` and `read_skill_file` tool definitions
- System prompt adds `<available_skills>` XML block

**SkillService** (`crates/server/src/services/skill.rs`):
- Business logic for CRUD operations
- SKILL.md parsing and validation
- ZIP archive extraction and validation
- Integration with capability listing

**SkillToolExecutor** (`crates/worker/src/skill_executor.rs`):
- Handles `activate_skill` tool calls
- Fetches skill content via gRPC
- Returns SKILL.md body as tool result
- Handles `read_skill_file` for archive-based skills

### gRPC Protocol

```protobuf
message SkillInfo {
    string id = 1;
    string name = 2;
    string description = 3;
    string instructions = 4;
    string source_type = 5;
}

message GetSkillByNameRequest {
    string name = 1;
    int64 org_id = 2;
}

message GetSkillByNameResponse {
    optional SkillInfo skill = 1;
}

message GetSkillFileRequest {
    string skill_name = 1;
    string file_path = 2;
    int64 org_id = 3;
}

message GetSkillFileResponse {
    optional string content = 1;
}
```

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

### Skills Settings Page

Located at `/settings/skills` in the UI. Components:

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

No skills are seeded by default. The registry starts empty, and users add skills as needed.

Optional: A "getting started" skill could be provided:

| Name | Description |
|------|-------------|
| `hello-world` | A simple example skill that demonstrates the Agent Skills format |

## Migration Strategy

New migration file: `NNN_add_skills.sql` (next available number after current migrations).

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
