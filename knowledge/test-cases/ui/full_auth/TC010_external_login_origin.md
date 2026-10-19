---
type: Test Case
title: "TC010: Full Auth - External Login Origin"
description: "Verify that protected routes can delegate the login page to a trusted remote origin while preserving a safe relative `return_to` continuation."
tags:
  - everruns
  - test-case
  - ui
  - full-auth
---
# TC010: Full Auth - External Login Origin

## Description

Verify that protected routes can delegate the login page to a trusted remote
origin while preserving a safe relative `return_to` continuation.

## Preconditions

- `AUTH_MODE=full`
- `AUTH_LOGIN_ORIGIN=https://id.example.com` is set identically for the server
  and UI runtimes
- Clean browser session with neither `access_token` nor `refresh_token` cookie

## Test Data

| Continuation | Expected |
| --- | --- |
| `/models/providers?tab=models` | Preserved relative path and query |
| `/oauth/authorize?client_id=test&response_type=code` | Backend continuation; full-page navigation |
| `https://example.invalid/leave`, `//example.invalid/leave`, slash followed by backslash | Rejected; remain on trusted origin |
| Slash followed by an encoded tab or newline and another slash | Rejected after query decoding |

## Steps

1. Navigate to `/models/providers?tab=models` on the app origin.
2. Observe the browser navigation target.
3. Repeat with an OAuth-style protected continuation:
   `/oauth/authorize?client_id=test&response_type=code`.
4. Inspect `GET /v1/auth/config` from the app API.
5. Complete login for the safe continuation; verify the selected route and query load. Repeat with the rejected continuation variants as the login page's `return_to`.
6. Remove `AUTH_LOGIN_ORIGIN`, restart the stack, clear both auth cookies, and repeat step 1.

## Expected Result

- Steps 1–2 use a full-page navigation to
  `https://id.example.com/login?return_to=%2Fmodels%2Fproviders%3Ftab%3Dmodels`;
  the Next.js client router is not used.
- Step 3 preserves the entire relative OAuth continuation in `return_to`.
- Step 4 returns `login_origin: "https://id.example.com"` and exposes no way
  for request or query input to override it.
- Step 6 preserves the original relative same-origin redirect byte-for-byte:
  `/login?return_to=%2Fmodels%2Fproviders%3Ftab%3Dmodels`.
- Absolute, protocol-relative, or backslash-prefixed `return_to` values remain
  rejected by the login page sanitizer.
