---
title: Skills Registry
description: API endpoints for managing organization-wide skill packages, create from SKILL.md or ZIP, validate, assign to agents as virtual capabilities.
---

The **Skills Registry** stores skills at the organization level. Registry skills persist across sessions and appear in the capability system as virtual capabilities with ID `skill:{uuid}`. They can be assigned to any agent like any other capability.

For workspace-scoped skills (drop a SKILL.md into a session's `/.agents/skills/`), see [Agent Skills](/features/skills/).

## API surface

| Method | Path | Description |
|---|---|---|
| POST | `/v1/skills` | Create from SKILL.md |
| POST | `/v1/skills/upload` | Create from ZIP archive |
| GET | `/v1/skills` | List |
| GET | `/v1/skills/{id}` | Get metadata |
| GET | `/v1/skills/{id}/content` | Get full content |
| PATCH | `/v1/skills/{id}` | Update |
| DELETE | `/v1/skills/{id}` | Delete |
| POST | `/v1/skills/validate` | Validate without creating |

## ID formats

| Source | ID | Example |
|---|---|---|
| Registry | `skill:{uuid}` | `skill:550e8400-e29b-41d4-a716-446655440000` |
| Workspace | Aggregated under `skills` capability |, |

## Dependencies

Registry skills automatically depend on `session_file_system` so bundled resources can be read after activation. You don't need to add it explicitly.

## Security

- Archive uploads are validated against path traversal, ZIP bombs, and size limits.
- Skill instructions are returned as tool results, not injected into the system prompt, they cannot bypass capability isolation.
- Skill names are unique per organization.
- Disabled skills are hidden from listings.

## Do something

- [Publish a skill to the registry](/how-to/publish-a-skill-to-the-registry/), full upload, validate, assign flow.
- [Package an agent skill](/how-to/package-a-skill/), author the SKILL.md.

## See also

- [Agent Skills](/features/skills/), the concept and workspace flow.
- [API reference](/api/), full request/response schemas.
