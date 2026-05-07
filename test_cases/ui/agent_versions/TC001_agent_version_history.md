# TC001 Agent Version History

## Description

Verifies that a user can save agent versions, compare changes, set a default version, roll back, fork, and configure an App version policy from the UI.

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
2. Open the agent detail page and select the Versions tab.
3. Save a version with summary `Initial version`.
4. Edit the agent prompt to the updated prompt.
5. Return to Versions and save a patch version with summary `Prompt update`.
6. Use Compare Versions to compare the first version to the second version.
7. Set the second version as Default.
8. Roll back to the first version and confirm the rollback dialog.
9. Fork the second version into `version-ui-agent-fork`.
10. Create or open an App using `version-ui-agent`, edit Configuration, and set Agent version to Pinned with the second version.

## Expected Result

- The Versions tab is visible only when the feature flag is enabled.
- Two saved versions appear with semantic labels and summaries.
- The diff shows the system prompt changing from the initial prompt to the updated prompt.
- The selected default version displays a Default badge.
- Rollback updates the editable agent draft and appends a rollback history entry.
- Fork creates a new agent with lineage from the selected version.
- The App configuration displays `pinned` with the selected version.
