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

Verifies that a user can save agent versions, compare changes, set a default version, pin a version to an endpoint, roll back, and fork from the UI.

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
8. Open the agent's **Integrations** tab, add a webhook endpoint, and open its editor. In the
   **Agent version** rail section set **Runs** to **A pinned version**, choose `0.1.0`, and save.
9. Expand the endpoint row on the Integrations tab.
10. Reopen the endpoint editor, set **Runs** back to **Agent default version**, and save.
11. Roll back to the first version and confirm the rollback dialog.
12. Fork the second version into `version-ui-agent-fork`.

## Expected Result

- The Version history menu item is visible only when the feature flag is enabled.
- Two saved versions appear with semantic labels and summaries.
- The diff shows the system prompt changing from the initial prompt to the updated prompt.
- The selected default version displays a Default badge.
- The pinned version picker offers only saved versions, not automatic draft snapshots.
- After pinning, the endpoint editor header and the expanded endpoint row show `Pinned to 0.1.0`
  even though `0.1.1` is the default; after unpinning the badge is gone.
- Rollback updates the editable agent draft and appends a rollback history entry.
- Fork creates a new agent with lineage from the selected version.
