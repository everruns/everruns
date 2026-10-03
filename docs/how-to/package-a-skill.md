---
title: Package and publish an agent skill
description: Author a SKILL.md, bundle scripts and references, use it from the session workspace, and publish it to the organization-wide Skills Registry so any agent can use it.
appliesTo: [platform, cloud]
---

Skills are portable instruction packages following the [Agent Skills](https://agentskills.io/) open spec. They use progressive disclosure: the agent sees only names and descriptions until it activates a skill, at which point the full instructions load.

This guide first packages a skill and uses it from one session's workspace, then publishes it to the Skills Registry so any agent in the organization can use it.

## SKILL.md format

Every skill is a directory containing a `SKILL.md` with YAML front matter:

```yaml
---
name: csv-analyzer
description: Analyze CSV files and generate summary reports.
metadata:
  category: data-processing
  version: "1.0"
---

# CSV Analyzer

## When to Use

Activate this skill when a user provides a CSV file and wants summary statistics.

## Instructions

1. Read the CSV file using the `read_file` tool
2. Run `scripts/analyze.py` via `bash`
3. Present findings to the user
```

Required front-matter fields:

| Field | Constraint |
|---|---|
| `name` | 1–64 chars, lowercase alphanumeric + hyphens |
| `description` | 1–1024 chars, describes when to activate |

Optional fields: `metadata`, `license`, `compatibility`.

## Bundle scripts and references

Skills can include arbitrary files. The agent accesses them via the session filesystem after activation:

```
/.agents/skills/csv-analyzer/
├── SKILL.md
├── scripts/
│   └── analyze.py
└── references/
    └── REFERENCE.md
```

The skill lives at `/workspace/.agents/skills/csv-analyzer/` in the session VFS. After activation, the agent reads its bundled files there with the regular file tools such as `read_file` and `list_directory`.

## Enable the skills capability

Add the built-in `skills` capability to the agent so it can discover and activate skills from the workspace:

```bash
curl -X POST http://localhost:9300/api/v1/agents \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Data Analyst",
    "capabilities": [
      { "ref": "skills" },
      { "ref": "session_file_system" }
    ]
  }'
```

`skills` depends on `session_file_system`; the platform pulls it in automatically.

## How activation looks to the agent

With the capability enabled, the system prompt includes an `<available_skills>` block (~100 tokens per skill):

```xml
<available_skills>
  <skill>
    <name>csv-analyzer</name>
    <description>Analyze CSV files and generate summary reports.</description>
  </skill>
</available_skills>
```

When the user's task matches, the agent calls `activate_skill`:

```json
{ "name": "activate_skill", "arguments": { "name": "csv-analyzer" } }
```

The tool returns the full SKILL.md instructions wrapped in `<skill>` tags. The agent now has the detailed instructions in context and can run the bundled scripts.

## Test the skill

1. Start a session with an agent that has the `skills` capability enabled.
2. Write the skill files to `/.agents/skills/<name>/` in the session.
3. Send a message that matches the skill's "When to Use" criteria.
4. Watch the event stream for an `activate_skill` tool call.

## Publish to the Skills Registry

The Skills Registry stores skills at the organization level. Registry skills persist across sessions and can be assigned to any agent as a capability with ID `skill:{uuid}`. When a session starts, the skill is mounted into `/.agents/skills/{name}/`, so the `skills` capability discovers it the same way as a workspace skill.

### From SKILL.md

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

### From a ZIP archive

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

### Validate first

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

### Assign to an agent

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

### Update or delete

```bash
# Update
curl -X PATCH http://localhost:9300/api/v1/skills/$SKILL_ID \
  -H "Content-Type: application/json" \
  -d '{"skill_md": "..."}'

# Delete
curl -X DELETE http://localhost:9300/api/v1/skills/$SKILL_ID
```

Deleting a skill hides it from capability listings. Agents that reference it via `ref: "skill:..."` will still resolve until you update them.

## See also

- [Agent Skills](/features/skills/): the concept, the registry API, and its security rules.
- [Agent Skills capability reference](/capabilities/agent-skills/): `list_skills` and `activate_skill`.
