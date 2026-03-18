---
title: Agent Skills
description: Discover and activate portable skill packages from the session workspace. Agents gain specialized knowledge and workflows by loading skill definitions at runtime.
---

| | |
|---|---|
| **ID** | `skills` |
| **Category** | Skills |
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

1. **Discovery** (~100 tokens) — `list_skills` returns only names and descriptions
2. **Activation** (<5000 tokens) — `activate_skill` loads the full SKILL.md instructions
3. **Resources** (on-demand) — bundled files accessible via [File System](/capabilities/file-system/) tools

## Use Cases

- **Project-specific workflows** — upload skills for your project's deployment, testing, or review processes
- **Specialized knowledge** — domain-specific instructions (e.g., security review checklists)
- **Reusable templates** — code generation patterns, documentation templates

## Example

Skills are uploaded to the session workspace:

```
/.agents/skills/
  deploy/
    SKILL.md          # Deployment instructions
    templates/
      k8s-deploy.yaml # Kubernetes template
  code-review/
    SKILL.md          # Review checklist and process
```

Agent discovers and uses them:

```
Agent:
  → list_skills()
  ← [{ name: "deploy", description: "Production deployment workflow" },
     { name: "code-review", description: "Code review checklist" }]

  → activate_skill({ name: "deploy" })
  ← { instructions: "## Deployment Process\n1. Run tests...", files: ["templates/k8s-deploy.yaml"] }
```

## Notes

- Skills are per-session (uploaded to session filesystem)
- Path traversal protection on skill names
- Invalid SKILL.md files are reported but don't block discovery of other skills
- For organization-wide skills, see the [Skills Registry](/features/skills-registry/)

## See Also

- [Agent Skills feature guide](/features/skills/) — detailed skills documentation
- [Skills Registry](/features/skills-registry/) — API-managed skills
- [AGENTS.md](/capabilities/agent-instructions/) — simpler alternative for project context
- [File System](/capabilities/file-system/) — upload skill files
- [Capabilities Overview](/capabilities/)
