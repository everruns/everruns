---
type: Test Case
title: "TC001: Organization Provider Account"
description: "Verify an administrator can manage an organization-owned sandbox provider account and bind it to a Sandbox Template."
tags:
  - everruns
  - test-case
  - ui
  - sandbox
---
# TC001: Organization Provider Account

## Description

Verify that provider credentials shared by an organization are managed from
the Sandboxes area and that a managed Sandbox Template can pin one exact
organization account.

## Preconditions

- Canonical local stack running with `AUTH_MODE=none`
- Organization administrator access
- Valid Daytona API key available for the test

## Steps

1. Open **Sandboxes → Provider Accounts**.
2. Verify Daytona and E2B appear as provider-neutral account cards. Verify
   Modal also appears when the deployment enables development-grade providers.
3. Add a Daytona account named `UI smoke Daytona`, enter its API key, and save.
4. Verify the account appears without exposing its API key.
5. Open **Sandboxes → Templates** and create or edit a Daytona template.
6. Set **Account source** to **Organization provider account** and select
   `UI smoke Daytona`.
7. Save, reopen the template, and verify the account choice is preserved.
8. Return to **Provider Accounts** and try to delete the account.
9. Verify deletion is refused while the active template references it.
10. Change the template to another account source or delete the template, then
    delete `UI smoke Daytona`.

## Expected Result

The account is visible only as non-secret metadata, the template persists its
exact account binding, and Everruns prevents deletion while an active template
or Sandbox lease depends on the account.

## Cleanup

Delete the temporary template and `UI smoke Daytona` account.
