# TC006: Memory Capability Config Store Picker

## Description

Verify that the Agent capability configuration replaces the raw `mst_…` text input for the `memory` capability with a structured store picker populated from the org's stores, and that the selected store is persisted as the capability config.

## Preconditions

- UI running and user authenticated
- At least two memory stores exist in the org (e.g. `team-knowledge` from TC001 and `org-default` from TC002)
- An agent exists that the user can edit, or a new agent can be created

## Test Data

| Field | Value |
|-------|-------|
| Capability | memory |
| Store selection 1 | Default store (empty value) |
| Store selection 2 | `team-knowledge` |
| Passive recall count | 3 |

## Steps

1. Navigate to an agent's edit page (Agents → pick an agent → Edit) or create a new agent
2. Open the **Capabilities** section and enable the **memory** capability if not already enabled
3. Open the memory capability's settings panel
4. Confirm a **Memory store** combobox is shown (not a free-text input) with placeholder "Default store"
5. Open the combobox and verify it lists at least: `Default store`, `team-knowledge`, `org-default (default)`
6. Pick `team-knowledge`
7. Set **Passive recall count** to `3`
8. Save the agent
9. Reload the agent edit page

## Expected Result

- Step 4: The memory capability uses the structured editor (combobox + numeric input), not the generic JSON form
- Step 5: All org stores are listed; the org's default store has a `(default)` suffix; a `Default store` option (sentinel) is the first choice
- Step 6: After picking `team-knowledge`, the trigger shows `team-knowledge` (without `(default)` suffix)
- Step 8: Save succeeds. Inspecting the agent's stored config (e.g. via API `GET /v1/agents/{id}`) shows `capabilities.memory.config = { "store": "mst_<id-of-team-knowledge>", "passive_recall_count": 3 }`
- Step 9: The reloaded form shows `team-knowledge` selected and `Passive recall count = 3`
- If the store is later deleted/archived from outside the form, the picker shows the destructive message "Selected store is no longer available in this organization."
