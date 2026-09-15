---
type: Test Case
title: "TC001: Capability discovery and detail"
description: "Verify that capability search and filters find the correct capability and that its detail page exposes status, contributions, dependencies, and usage."
tags:
  - everruns
  - test-case
  - ui
  - capabilities
---
# TC001: Capability discovery and detail

## Description

Verify that capability search and filters find the correct capability and that its detail page exposes status, contributions, dependencies, and usage.

## Preconditions

- The full stack is running and the user is signed in.
- Available, coming-soon, and deprecated capabilities exist.
- One capability has a system prompt, tool definitions, dependencies, and agent or harness usage.

## Test Data

| Field | Value |
|---|---|
| List route | `/capabilities` |
| Search value | The known capability name or ID |
| Category | The known capability category |

## Steps

1. Open `/capabilities` and verify total and status counts.
2. Search by the known name, description term, ID, and category in turn; confirm each query finds the same capability.
3. Select its category and status filters and verify unrelated cards disappear.
4. Clear filters and open the capability card.
5. Confirm `/capabilities/{capabilityId}` shows the localized name and description, status, category, agent and harness counts, and documentation link when configured.
6. Verify the system-prompt addition is rendered as markdown.
7. Verify each tool shows name, description, approval policy, and parameter schema.
8. Follow a dependency link and confirm it opens the dependency's detail page.
9. Open a capability without prompt or tools and confirm the **No contributions** state.

## Expected Result

- Search covers names, descriptions, IDs, categories, and localizations.
- Status and category filters compose and can be cleared.
- Detail data belongs to the selected capability and matches list usage counts.
- Prompt, tools, policies, parameters, and dependencies remain readable and navigable.
- Capabilities with no contributions show an explicit empty state.
