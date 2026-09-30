---
type: Test Case
title: "TC001: Agent Version History"
description: "Verifies that a user can save agent versions, compare changes, set a default version, roll back, and fork from the UI."
tags:
  - everruns
  - test-case
  - ui
  - agent-versions
---
# TC001: Agent Version History

## Description

Verifies that a user can save agent versions, compare changes, set a default version, roll back, and fork from the UI.

## Preconditions

- `FEATURE_AGENT_VERSIONS=true` or development grade enabled.
- User is signed in to an organization with permission to manage agents and apps.
- At least one harness exists.

## Test Data

| Field | Value |
|---|---|
| Agent name | `version-ui-agent` |
| Initial prompt | `You are version one.` |
| Updated prompt | `You are version two.` |
| Fork name | `version-ui-agent-fork` |

## Steps

1. Open Agents and create `version-ui-agent` with the initial prompt.
2. Open the agent page and select **Version history** in the header overflow menu.
3. Save a version with summary `Initial version`.
4. Edit the agent prompt to the updated prompt.
5. Reopen Version history and save a patch version with summary `Prompt update`.
6. Use Compare Versions to compare the first version to the second version.
7. Set the second version as Default.
8. Roll back to the first version and confirm the rollback dialog.
9. Fork the second version into `version-ui-agent-fork`.

## Expected Result

- The Version history menu item is visible only when the feature flag is enabled.
- Two saved versions appear with semantic labels and summaries.
- The diff shows the system prompt changing from the initial prompt to the updated prompt.
- The selected default version displays a Default badge.
- Rollback updates the editable agent draft and appends a rollback history entry.
- Fork creates a new agent with lineage from the selected version.

Pinning a version is deliberately not covered. The step that set it went through the `/apps`
configuration page, which was removed with the App UI, and nothing replaced it — the policy is
still honoured wherever it is stored but cannot be set from any surface (EVE-1139). Re-add the
coverage with the write path rather than leaving a step no tester can perform.
