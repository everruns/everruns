---
type: Test Case
title: "TC007: Setup Rejects an API Key the Provider Refuses"
description: "Verify that org setup checks the entered provider API key with the provider before storing it: a rejected key is refused inline with no provider created, while a valid key completes setup."
tags:
  - everruns
  - test-case
  - ui
  - org-creation
---
# TC007: Setup Rejects an API Key the Provider Refuses

## Description

Verify that the org setup page checks the entered provider API key against the
provider before storing it: a key the provider rejects is refused inline and no
provider is created, while a valid key completes setup.

## Preconditions

- User just created a new organisation and was redirected to `/orgs/<orgId>/setup`
- The four provisioning steps have finished animating, so the provider form is visible
- The stack has outbound network access to the selected provider

## Test Data

| Field | Value |
|---|---|
| Provider | OpenAI |
| Invalid API key | `sk-invalid-0000000000000000000000000000` |
| Valid API key | A working OpenAI key |

## Steps

1. Select the "OpenAI" provider card
2. Enter the invalid API key and click "Finish setup"
3. Observe the button label while the check runs, then the resulting message
4. Open Settings → Providers in another tab and confirm no OpenAI provider was created
5. Return to setup, replace the key with the valid API key, and click "Finish setup"

## Expected Result

- While the check runs the primary button reads "Checking key..." and both it and
  "Skip for now" are disabled
- The invalid key produces an inline error: "OpenAI rejected this API key. Check the key and try again."
- The user stays on the Configure step; Settings → Providers shows no new OpenAI provider
- The valid key proceeds: the button reads "Configuring...", then the Done step appears
  ("You're all set.") and the copy states the workspace is connected to a model provider
- Settings → Providers now lists the OpenAI provider
