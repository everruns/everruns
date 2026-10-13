---
type: Test Case
title: "TC002: Personal Access Token - Read-only and Revocation"
description: "Verify that read-only tokens can read authorized organization data, reject writes and token management, and stop authenticating after revocation."
tags:
  - everruns
  - test-case
  - ui
  - personal-access-tokens
---
# TC002: Personal Access Token - Read-only and Revocation

## Description

Verify that read-only tokens can read authorized organization data, reject writes and token management, and stop authenticating after revocation.

## Preconditions

- Authenticated stack and an account with access to two test organizations.
- A disposable agent exists in each organization; record their IDs and original names.
- API checks use only the PAT and explicit `X-Org-Id`; omit session cookies so they cannot mask credential failures.

## Test Data

| Field | Value |
| --- | --- |
| Token name | `manual-read-only-<run-id>` |
| Access | Read-only |
| Expiration | 7 days |
| Attempted agent name | `must-not-save-<run-id>` |

## Steps

1. Open Settings > Personal access tokens and create the token with **Read-only** selected.
2. Inspect the creation request and one-time result. Copy the secret privately, close the dialog, and reload the page.
3. Read agents in both organizations using the token and the respective organization header.
4. Try to create an agent, change an existing agent's name, and send a session message using the read-only token.
5. Try to list or create personal access tokens using the read-only token.
6. Using the browser session, re-read the existing agents and sessions; verify no denied operation changed data or started work.
7. Revoke the token from the settings page. Repeat a previously successful read with the same secret.

## Expected Result

- Creation sends `scopes: ["read"]`; the list shows Read-only and the selected expiry. The full secret is shown only in the creation result, never on list/reload.
- Authorized reads succeed independently in both orgs. Read-only still exposes account-wide readable data; the copy says so.
- Writes and token-management requests return 403 with `code: insufficient_scope` and a Bearer insufficient-scope challenge.
- Denied operations have no side effects.
- Revocation removes the token from the list and subsequent token authentication returns 401.
