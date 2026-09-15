---
type: Test Case
title: "TC001: Skill create, inspect, archive, and delete"
description: "Verify that a skill can be created from SKILL.md, found and inspected, archived, and permanently deleted only when policy permits."
tags:
  - everruns
  - test-case
  - ui
  - skills
---
# TC001: Skill create, inspect, archive, and delete

## Description

Verify that a skill can be created from SKILL.md, found and inspected, archived, and permanently deleted only when policy permits.

## Preconditions

- The full stack is running and the `skills` feature is enabled.
- The user can create and archive skills.
- One user has the `skill.dangerous` permission and one does not.

## Test Data

| Field | Value |
|---|---|
| Route | `/skills` |
| Name | `eve-995-manual-skill` |
| Description | `Manual UI coverage skill` |
| Body | `Follow the manual coverage instructions.` |

## Steps

1. Open `/skills`, click **Add Skill**, and replace the template with valid frontmatter and the test body.
2. Create the skill and search for its name.
3. Verify the row shows active status, source type, version, description, allowed tools or license when supplied, and usage.
4. Open `/skills/{skillId}` from the row and verify the entity ID, metadata, status, source, version, dates, and usage.
5. Open **Instructions** and confirm frontmatter and rendered SKILL.md body match the submitted content.
6. Open **Files** and verify the read-only tree contains `SKILL.md`.
7. Return to `/skills`, archive the skill, select **Archived**, and confirm it appears there but not under **Active**.
8. As a user without `skill.dangerous`, confirm the archived row has no permanent delete action.
9. As a user with `skill.dangerous`, click **Delete**, cancel the dialog, and confirm the skill remains.
10. Delete again, approve the dialog, and confirm the skill is removed or represented only by deleted tombstones where referenced.

## Expected Result

- Valid SKILL.md content creates one searchable active skill.
- Detail tabs preserve metadata, instructions, and read-only files.
- Archive moves the skill from Active to Archived.
- Permanent deletion is policy-gated and requires confirmation.
- Canceling deletion is non-destructive; approving it removes the archived resource.
