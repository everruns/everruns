---
title: Agent Skills
description: Discover and activate portable skill packages from the session workspace at runtime.
appliesTo: [framework, platform, cloud]
# The feature page shares this title; keep the <title> distinct for search results.
head:
  - tag: title
    content: "Agent Skills Capability | Everruns"
---

| | |
|---|---|
| **ID** | `skills` |
| **Category** | Core |
| **Features** | None |
| **Dependencies** | [`session_file_system`](/capabilities/file-system/) |

Discover and activate skills from `/.agents/skills/` in the session filesystem. Skills are portable instruction packages following the [Agent Skills](https://agentskills.io/) open specification.

## Tools

### `list_skills`

Scan `/.agents/skills/` for available skills. Returns names and descriptions only (~100 tokens per skill).

### `activate_skill`

Load a skill's full instructions by name.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `name` | string | yes | Skill name (directory name under `/.agents/skills/`) |

Returns: full SKILL.md content and list of bundled files.

## How It Works

Skills use progressive disclosure to keep context efficient:

1. **Discovery** (~100 tokens), `list_skills` returns only names and descriptions
2. **Activation** (<5000 tokens), `activate_skill` loads the full SKILL.md instructions
3. **Resources** (on-demand), bundled files accessible via [File System](/capabilities/file-system/) tools

## Workspace layout

```
/.agents/skills/
  deploy/
    SKILL.md
    templates/
      k8s-deploy.yaml
  code-review/
    SKILL.md
```

## Notes

- Skills are per-session (uploaded to session filesystem)
- Path traversal protection on skill names
- Invalid SKILL.md files are reported but don't block discovery of other skills
- For organization-wide skills, see the [Skills Registry](/features/skills/#skills-registry)

## See Also

- [Agent Skills feature guide](/features/skills/), detailed skills documentation
- [Skills Registry](/features/skills/#skills-registry), API-managed skills
- [AGENTS.md](/capabilities/agent-instructions/), simpler alternative for project context
- [File System](/capabilities/file-system/), upload skill files
- [Capabilities Overview](/capabilities/)

## Prompt caching during installation

Skill discovery instructions stay in the system prompt. When installation or
an edit changes the available skills, the inventory is delivered as a later
conversation update. Unchanged inventories add no repeated update; removed
inventories receive an explicit clearing update. Updates survive worker
restarts through stored conversation metadata.

The same behavior applies to multi-scope skills and optional inspection
reports. `list_skills` still returns the current full catalog, and
`activate_skill` loads the selected instructions through the authorized
filesystem. Sessions carrying these updates use local history reduction
rather than opaque provider-managed checkpoints.
