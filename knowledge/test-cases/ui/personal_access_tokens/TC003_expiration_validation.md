---
type: Test Case
title: "TC003: Personal Access Token - Expiration Boundaries"
description: "Verify expiration presets, custom-day validation, explicit non-expiring choice, and dialog reset without silently broadening token access."
tags:
  - everruns
  - test-case
  - ui
  - personal-access-tokens
---
# TC003: Personal Access Token - Expiration Boundaries

## Description

Verify expiration presets, custom-day validation, explicit non-expiring choice, and dialog reset without silently broadening token access.

## Preconditions

- Authenticated stack and a disposable account below the token limit.
- Network panel records token creation payloads; revoke all accepted test tokens afterward.

## Test Data

| Choice | Expected validity |
| --- | --- |
| Default | 90 days; Full access |
| Custom 1 / 3650 | Valid inclusive boundaries |
| Custom blank / 0 / 3651 / -1 / 1.5 / text | Invalid |
| Never expires | Explicit checkbox; expiration omitted |

## Steps

1. Open Create token and inspect defaults.
2. Enter a name, choose Custom, and try each invalid value. Check Create token and Network.
3. Create separate tokens at the lower and upper valid boundaries; inspect `expires_in_days` and displayed expiry.
4. Reopen the dialog, choose **Read-only**, then explicitly choose **Never expires**. Create and inspect the result.
5. Reopen the dialog, alter access and expiry, cancel, and open it again.
6. Verify the saved non-expiring token is still Read-only; inspect the list response for `expires_at`.

## Expected Result

- Invalid custom values disable creation and send no request.
- Boundaries send exactly 1 and 3650; list expiry agrees with the selected duration.
- Never expires is a separate intentional choice and creates no expiry; it does not change the chosen scope.
- Cancel/reopen resets the dialog to Full access, 90 days, empty name/custom input, and Never expires unchecked.
- Existing tokens do not change when the create dialog resets.
