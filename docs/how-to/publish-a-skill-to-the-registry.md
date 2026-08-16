---
title: Publish a skill to the registry
description: Upload a SKILL.md or ZIP archive to the organization-wide Skills Registry so any agent can use it as a capability.
---

The Skills Registry stores skills at the organization level. Registry skills persist across sessions and can be assigned to any agent as a capability with ID `skill:{uuid}`.

For workspace-scoped skills, see [Package an agent skill](/how-to/package-a-skill/) instead.

## From SKILL.md

If your skill is a single Markdown file, post it directly:

```bash
curl -X POST http://localhost:9300/api/v1/skills \
  -H "Content-Type: application/json" \
  -d '{
    "skill_md": "---\nname: hello-world\ndescription: A simple greeting skill.\n---\n\n# Hello World\n\nGreet the user warmly."
  }'
```

The response includes the skill ID:

```json
{ "id": "skill_550e8400-e29b-41d4-a716-446655440000", "name": "hello-world", ... }
```

## From a ZIP archive

For skills with bundled scripts, references, or assets:

```bash
curl -X POST http://localhost:9300/api/v1/skills/upload \
  -F "file=@csv-analyzer.zip"
```

Archive layout:

```
csv-analyzer/
├── SKILL.md
├── scripts/analyze.py
└── references/REFERENCE.md
```

The top-level directory name is informational; the skill `name` comes from the SKILL.md front matter and must be unique per organization.

## Validate first

Validate without creating:

```bash
curl -X POST http://localhost:9300/api/v1/skills/validate \
  -H "Content-Type: application/json" \
  -d '{"skill_md": "---\nname: my-skill\ndescription: Does things.\n---\n\n# Instructions"}'
```

Response:

```json
{ "valid": true, "name": "my-skill", "description": "Does things.", "warnings": [] }
```

## Assign to an agent

Registry skills appear in the capability system as virtual capabilities with ID `skill:{uuid}`:

```bash
curl -X POST http://localhost:9300/api/v1/agents \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Analyst Agent",
    "capabilities": [
      { "ref": "skill:550e8400-e29b-41d4-a716-446655440000" },
      { "ref": "session_file_system" }
    ]
  }'
```

`session_file_system` is pulled in automatically as a dependency.

## Update or delete

```bash
# Update
curl -X PATCH http://localhost:9300/api/v1/skills/$SKILL_ID \
  -H "Content-Type: application/json" \
  -d '{"skill_md": "..."}'

# Delete
curl -X DELETE http://localhost:9300/api/v1/skills/$SKILL_ID
```

Deleting a skill hides it from capability listings. Agents that reference it via `ref: "skill:..."` will still resolve until you update them.

## Security notes

- Archive uploads are validated for path traversal, ZIP bombs, and size limits.
- Skill instructions are returned as tool results, not injected into the system prompt, they don't bypass capability isolation.
- Skill names are unique per organization.
- Disabled skills are hidden from listings.

## See also

- [Skills Registry feature](/features/skills-registry/)
- [Package an agent skill](/how-to/package-a-skill/), author the SKILL.md.
