---
type: Test Case
title: "TC001: Harness Inheritance Relationships"
description: "Verify current harness parent relationships, deeper ancestry, and locally declared capabilities across grid, list, and narrow layouts."
tags:
  - everruns
  - test-case
  - ui
  - harnesses
---
# TC001: Harness Inheritance Relationships

## Description

Verify current harness parent relationships, deeper ancestry, and locally declared capabilities across grid, list, and narrow layouts.

## Preconditions

- Canonical stack with current built-in harnesses.
- An owner-created child of Bashkit Worker with a long display name and a harmless locally declared capability.

## Test Data

| Harness | Parent |
| --- | --- |
| Base | None |
| Conversation | Base |
| Worker | Base |
| Bashkit Worker | Worker |
| Sandbox Worker | Worker |
| Custom child | Bashkit Worker |

## Steps

1. Open Harnesses in grid view and verify the table's direct parents.
2. Inspect the custom child's ancestry tooltip; it should show Base → Worker → Bashkit Worker → child.
3. Inspect locally declared capability chips and compare them with the effective preview.
4. Switch to list view and repeat; follow the parent link and return.
5. Repeat at a narrow viewport using keyboard navigation to links and tooltips.

## Expected Result

- Non-root cards identify and link their immediate parent; Base does not show a false parent.
- Deeper ancestry is available without replacing the direct-parent label.
- Declared capabilities are distinguishable from inherited effective configuration.
- Long names remain readable through accessible labels/tooltips without breaking grid/list or narrow layouts.

## Source Contract

[Canonical harness levels](../../../../crates/server/src/harnesses/levels.rs).
