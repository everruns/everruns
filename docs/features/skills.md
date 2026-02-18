---
title: Agent Skills
description: Portable instruction packages that extend agent capabilities with specialized knowledge and workflows
---

Agent Skills are portable instruction packages following the [Agent Skills](https://agentskills.io/) open specification. They enable agents to discover and activate specialized knowledge on demand, keeping the agent's context efficient through progressive disclosure.

## Overview

Skills work through a three-stage progressive disclosure model:

1. **Discovery** (~100 tokens): Only skill names and descriptions are loaded at startup
2. **Activation** (<5000 tokens): Full instructions loaded when the agent calls `activate_skill`
3. **Resources** (on-demand): Bundled files accessed through the session filesystem

## SKILL.md Format

Every skill contains a `SKILL.md` file with YAML frontmatter and markdown instructions:

```yaml
---
name: csv-analyzer
description: Analyze CSV files and generate summary reports.
license: MIT
compatibility: Python 3.10+
metadata:
  category: data-processing
  version: "1.0"
allowed-tools: read_file write_file virtual_bash
---

# CSV Analyzer

## When to Use

Activate this skill when a user provides a CSV file and wants summary statistics.

## Instructions

1. Read the CSV file using the `read_file` tool
2. Run the analysis script
3. Present findings to the user
```

### Required Fields

| Field | Description |
|-------|-------------|
| `name` | 1-64 characters, lowercase alphanumeric + hyphens only |
| `description` | 1-1024 characters describing when to use the skill |

### Optional Fields

| Field | Description |
|-------|-------------|
| `license` | License identifier (e.g., MIT, Apache-2.0) |
| `compatibility` | Environment requirements (e.g., Python 3.10+) |
| `metadata` | Arbitrary key-value pairs |
| `allowed-tools` | Space-delimited tool names (experimental) |

## Creating Skills

### Via API (Registry-Based)

Create a skill from a SKILL.md file:

```bash
curl -X POST http://localhost:9000/v1/skills \
  -H "Content-Type: application/json" \
  -d '{
    "skill_md": "---\nname: hello-world\ndescription: A simple greeting skill.\n---\n\n# Hello World\n\nGreet the user warmly."
  }'
```

### Via ZIP Archive

For skills with bundled scripts, references, or assets:

```bash
curl -X POST http://localhost:9000/v1/skills/upload \
  -F "file=@csv-analyzer.zip"
```

Archive structure:
```
csv-analyzer/
├── SKILL.md
├── scripts/analyze.py
└── references/REFERENCE.md
```

### Filesystem Discovery (Built-in `skills` Capability)

Skills placed in the session filesystem at `/.agents/skills/` are automatically discovered when the built-in `skills` capability is enabled on an agent. This capability provides `list_skills` and `activate_skill` tools for VFS-based discovery.

```
/.agents/skills/
├── hello-world/
│   └── SKILL.md
├── csv-analyzer/
│   ├── SKILL.md
│   └── scripts/analyze.py
```

## Skills as Capabilities

Skills integrate into the capability system as virtual capabilities, appearing alongside built-in and MCP capabilities.

### Capability ID Format

- **Registry skills**: `skill:{uuid}` (e.g., `skill:550e8400-e29b-41d4-a716-446655440000`)
- **Filesystem skills**: Aggregated under `skills` capability ID

### Assigning to Agents

```bash
curl -X POST http://localhost:9000/v1/agents \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Analyst Agent",
    "capabilities": [
      { "ref": "skill:550e8400-e29b-41d4-a716-446655440000", "config": {} },
      { "ref": "session_file_system", "config": {} }
    ]
  }'
```

Skills automatically depend on `session_file_system` for reading bundled files.

## How Activation Works

When a skill is assigned to an agent, the system prompt includes an `<available_skills>` XML block:

```xml
<available_skills>
  <skill>
    <name>csv-analyzer</name>
    <description>Analyze CSV files and generate summary reports.</description>
  </skill>
</available_skills>

When a user's task matches an available skill, activate it by using the
`activate_skill` tool with the skill name.
```

The agent then uses the `activate_skill` tool to load full instructions:

```json
{
  "name": "activate_skill",
  "arguments": { "name": "csv-analyzer" }
}
```

The tool returns the complete SKILL.md instructions wrapped in `<skill>` tags. For archive-based skills, bundled files are mounted into the session VFS at `/skills/{name}/`.

## API Reference

| Method | Path | Description |
|--------|------|-------------|
| POST | `/v1/skills` | Create skill from SKILL.md |
| POST | `/v1/skills/upload` | Create skill from ZIP archive |
| GET | `/v1/skills` | List all skills |
| GET | `/v1/skills/{id}` | Get skill metadata |
| GET | `/v1/skills/{id}/content` | Get full skill content |
| PATCH | `/v1/skills/{id}` | Update skill |
| DELETE | `/v1/skills/{id}` | Delete skill |
| POST | `/v1/skills/validate` | Validate SKILL.md without creating |

## Validation

Use the validation endpoint to check a SKILL.md before creating:

```bash
curl -X POST http://localhost:9000/v1/skills/validate \
  -H "Content-Type: application/json" \
  -d '{"skill_md": "---\nname: my-skill\ndescription: Does things.\n---\n\n# Instructions"}'
```

Response:
```json
{
  "valid": true,
  "name": "my-skill",
  "description": "Does things.",
  "warnings": []
}
```

## Security

- Archive uploads are validated for path traversal, zip bombs, and size limits
- Skill instructions are returned as tool results (not injected into system prompt)
- Skill names are unique per organization
- Disabled skills are hidden from capability listings
- See the [Threat Model](/specs/threat-model/) for detailed security analysis (TM-TOOL-010 through TM-TOOL-014)
