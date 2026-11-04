---
type: Test Case
title: "TC001: Automatic session titles - Model defaults and explicit naming"
description: Verify independent small-model selection, opt-in automatic naming, and explicit-title priority.
tags: [ui, sessions, models]
---

# TC001: Automatic session titles - Model defaults and explicit naming

## Description

Verify independent small-model selection, opt-in automatic naming, and explicit-title priority.

## Preconditions

Canonical local stack using `AUTH_MODE=none` (anonymous development admin), or an
admin member authenticated in full mode. Two enabled shared chat models with
valid organization provider credentials. Record organization, model, and fixture
session IDs and each session's event cursor. Use fresh sessions for each policy.

## Test Data

| Value | Purpose |
|---|---|
| Main chat model | Agent/session model |
| Small chat model | Independent organization default |
| Plan a database migration | Substantive first request |
| My migration plan | Explicit title |

## Steps

1. Open Models → Defaults. Select Small model; reload and verify it persists
   while Default model is unchanged. Clear it, reload, and verify inheritance.
2. Select Small model again. Enable Automatic session titles on a fresh agent
   and open its settings. Verify organization default, Use session model, and
   explicit model/provider selections persist across saving and reloading.
3. With default configuration and no Session title tool, create a fresh session
   and send the substantive request. Wait at most 60 seconds for an automatic
   title and a new `session.title.updated` event after the recorded cursor.
   Inspect the title-purpose generation event's model/provider and usage.
4. Rename the session to the explicit title, then send follow-ups. After the
   documented cooldown and sufficient new messages, verify the explicit title
   persists and no further title-purpose requests were admitted for those turns.
5. In a fresh session with automatic naming disabled, send the same request and
   verify no title-purpose generation or automatic title event appears within
   60 seconds. Explicit naming through the existing tool remains available.
6. Disable the selected Small model. Verify its saved selection and repair
   warning remain visible; a new opted-in session does not generate a title on
   another model. Restore the model/default after the test.

## Expected Result

Small model is labeled directly in Chat defaults. It is independent of main and
Decisions defaults. Automatic naming works without agent tools, uses the chosen
provider, and reports usage. Explicit titles survive later eligible turns.
Unavailable selected models never redirect title generation to another account.

See [Session titles](../../../runtime-resources/session-titles.md) for the
admission and ownership contract and linked automated concurrency coverage.
