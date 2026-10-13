---
type: Test Case
title: "TC004: Verify Harnesses Exist After Org Setup"
description: "Verify that built-in harnesses are provisioned for the new organisation by navigating to the Harnesses page after setup."
tags:
  - everruns
  - test-case
  - ui
  - org-creation
---
# TC004: Verify Harnesses Exist After Org Setup

## Description

Verify that built-in harnesses are provisioned for the new organisation by navigating to the Harnesses page after setup. Harnesses now use addressable names (slugs) with separate display names.

## Preconditions

- User just completed org setup (all four steps show checkmarks)

## Steps

1. On the setup page, click "Skip for now" or finish provider setup and click "Continue"
2. Navigate to the Harnesses page via the sidebar
3. Review the list of harnesses displayed

## Expected Result

- The post-setup application loads without errors
- Harnesses shows the current built-ins: `base`, `conversation`, `worker`, `bashkit-worker`, and `sandbox-worker`; each links to the corresponding detail. Generic and Worker Base appear only when Show deprecated is enabled.
- Each harness card shows a "Built-in" badge
- Each harness card is clickable and links to a detail page

## Test Data

Use the newly created org ID from setup; existing harnesses in another selected org cannot satisfy this case.
