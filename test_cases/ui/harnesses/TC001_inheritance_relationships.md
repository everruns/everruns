# TC001: Harness Inheritance Relationships

## Description

Verify harness cards identify their direct parent, expose deeper ancestry, and distinguish locally
declared capabilities from inherited effective configuration in grid and list layouts.

## Preconditions

- DB-backed stack is running with the built-in `base`, `generic`, and `platform-chat` harnesses
- A custom child of `platform-chat` exists with a long display name and a locally declared capability

## Test Data

| Harness | Parent | Expected card relationship |
|---------|--------|----------------------------|
| Base | None | No inheritance row |
| Generic | None | No inheritance row |
| Platform Chat | Generic | `Inherits from Generic` |
| Long custom child | Platform Chat | `Inherits from Platform Chat`; tooltip shows Generic → Platform Chat → child |

## Steps

1. Open `/harnesses` at desktop width and select grid view.
2. Verify the built-in and custom cards against the table above.
3. Focus the custom child's branch icon and inspect the ancestry tooltip.
4. Verify capability chips are introduced by the label `Declared capabilities`.
5. Select list view and repeat the relationship checks.
6. Set the viewport to a mobile width and repeat grid/list checks.
7. Follow a parent relationship link, then use keyboard navigation to focus the relationship link
   and branch tooltip trigger.

## Expected Result

- Every non-root card names and links its direct parent; built-in status remains a separate badge.
- Root cards do not show a redundant root label.
- Multi-level ancestry is available from the branch tooltip without replacing the direct-parent label.
- Long names truncate without overflowing and remain available through the link title/tooltip.
- Capability chips are clearly described as locally declared, not effective inherited capabilities.
- Grid/list and desktop/mobile layouts remain readable and keyboard-operable.
