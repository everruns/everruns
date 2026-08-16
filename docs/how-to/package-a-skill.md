---
title: Package an agent skill
description: Author a SKILL.md, bundle scripts and references, and place it in the session workspace so agents can discover and activate it on demand.
---

Skills are portable instruction packages following the [Agent Skills](https://agentskills.io/) open spec. They use progressive disclosure: the agent sees only names and descriptions until it activates a skill, at which point the full instructions load.

This guide creates a skill in the session workspace. To share a skill across agents organization-wide, see [Publish a skill to the registry](/how-to/publish-a-skill-to-the-registry/).

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

After activation, bundled files mount at `/skills/csv-analyzer/` in the session VFS and the agent reads them with the existing `read_file` / `list_files` tools.

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

## See also

- [Agent Skills feature](/features/skills/)
- [Skills Registry](/features/skills-registry/), share skills across the organization.
- [Publish a skill to the registry](/how-to/publish-a-skill-to-the-registry/)
