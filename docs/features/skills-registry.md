---
title: Skills Registry
description: API-managed skill registry for creating, uploading, and sharing skills across your organization
---

The Skills Registry provides API endpoints for managing organization-wide skills. Registry skills persist across sessions and can be assigned to any agent as capabilities.

For an introduction to skills and the workspace-based workflow, see [Agent Skills](/features/skills/).

## Creating Skills

### From SKILL.md

Create a skill from a SKILL.md file:

```bash
curl -X POST http://localhost:9000/v1/skills \
  -H "Content-Type: application/json" \
  -d '{
    "skill_md": "---\nname: hello-world\ndescription: A simple greeting skill.\n---\n\n# Hello World\n\nGreet the user warmly."
  }'
```

### From ZIP Archive

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

## Skills as Capabilities

Registry skills integrate into the capability system as virtual capabilities, appearing alongside built-in and MCP capabilities.

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
- See the [Threat Model](https://github.com/everruns/everruns/blob/main/specs/threat-model.md) for detailed security analysis (TM-TOOL-010 through TM-TOOL-014)
