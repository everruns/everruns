---
type: Decision
title: "Entity Actions Menu"
description: "One header overflow menu on every entity page, for secondary record functions (history, manager notes) and lifecycle actions, in a fixed order."
tags:
  - everruns
  - ui
  - patterns
---

# Entity Actions Menu

Status: implemented (`apps/ui/src/components/entity-actions/`) for the record functions of
[Change Reasons and Manager Context](../execution/change-reasons-and-manager-context.md), on the
agent, harness, skill, provider, knowledge index, memory, observer, eval and virtual-user pages.
The bottom-sheet presentation on narrow screens and the page guard are not built yet.

## Problem

Entity pages keep growing secondary functions: copy, export, history, manager
notes, archive, delete. The [agent page](agent-page.md) and the
[harness page](harness-page.md) use this menu. Skill, eval, and virtual-user
pages still place delete and similar actions differently, and other pages have
none. Adding History and Manager notes as tabs or panels on every page would
clutter the part of the page people use most, for functions they use rarely.

## Decision

Every entity detail page has exactly one **actions menu**: a `⋯` icon button at
the right end of the page header, after the primary actions. It is the only
home for secondary functions. Primary actions (Save, Test in Playground) never
move into it.

### Fixed groups, fixed order

Items appear in three groups separated by dividers, always in this order, and
a group with no visible items is omitted with its divider:

1. **Entity actions**: what only this kind does (agent: Copy, Export as zip,
   Export as markdown). Ordered by frequency.
2. **Record**: functions every kind shares, in this order: **History**,
   **Manager notes**. A person who learns them on one page finds them in the
   same place on every page.
3. **Lifecycle**: Archive, then Delete. Delete is styled destructive and always
   last.

### Behavior

- **Items open, they do not navigate.** An item opens a side sheet (History,
  Manager notes) or a confirm dialog (Archive, Delete) over the page, so the
  person keeps their place. Leaving the page is a link, not a menu item.
- **The open sheet is part of the address** (`?sheet=history`,
  `?sheet=notes`), so a refresh or a shared link reopens it. Retired
  addresses map to the new sheet (`?tab=versions` opens History).
- **Hidden, not disabled, when not permitted.** An item the viewer can never
  use (Manager notes for a reader, Delete without permission) is not shown.
  An item that is temporarily unavailable (Export while one runs) is disabled
  with its reason as a tooltip.
- **No badges on the trigger.** The menu never signals state. Where state
  matters it is shown where it matters: an entity with manager notes shows one
  muted line in the header in edit mode only.
- **Accessible by default.** The trigger is labelled "More actions for
  {entity name}", the menu is keyboard navigable with arrow keys, and Escape
  returns focus to the trigger. On narrow screens the same menu opens as a
  bottom sheet; no item is desktop-only.
- **Small.** More than eight items means a function belongs somewhere else:
  in a More row or sheet on the page, or on a page of its own.

### One component

`EntityActionsMenu` takes the entity ref, its kind, the viewer's permissions
and the kind's own entity actions, and renders the Record and Lifecycle groups
itself. A page passes only what is specific to it, so a new entity page gets
History, Manager notes, Archive and Delete in the right order by adding one
component. The component is listed in the UI component gallery
(`apps/ui/src/app/dev/ui-components`).

## Rejected

- **Tabs for History and Manager notes.** Tabs are for primary views of the
  entity; these are occasional lookups and would add a tab to every page.
- **A sidebar or always-visible panel.** Costs width on every visit for a
  function used on a few.
- **Per-page placement.** The current state on harness, skill and eval pages:
  each page invents its own spot, and a person cannot predict where delete is.
- **A badge on the trigger when notes exist.** A dot on `⋯` reads as an alert
  and says nothing about what is inside.

## Testing

- A component test renders every group combination and asserts order,
  dividers, hidden items per permission set, and destructive styling on Delete.
- A guard lists entity detail pages and fails when one renders a delete or
  archive control outside `EntityActionsMenu`.
- Playwright smoke: open the menu on the agent page by keyboard, open History
  and Manager notes, check the address, reload, and see the sheet reopen.
