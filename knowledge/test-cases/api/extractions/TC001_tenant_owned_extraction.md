---
type: Test Case
title: "TC001: Tenant-owned extraction"
description: "Verify extraction requests use only an authorized organization's enabled model and stored provider credentials."
tags: [everruns, test-case, api, extractions]
---
# TC001: Tenant-owned extraction

## Description

Verify extraction requests use only an authorized organization's enabled model and stored provider credentials.

## Preconditions

- Canonical database-backed stack with full authentication for authorization checks.
- Two organizations and owner, member and read-only owner credentials.
- An enabled BYOK extraction model, an inactive provider, and a disabled model.
- Synthetic document only; provider fixture for offline tests or explicit spending approval for live qualification.

## Steps

1. POST `/v1/extractions` with a configured `model_id` and invoice text/field input. Verify the native extraction endpoint receives the stored key and catalog model, not a chat request.
2. Verify result values, Unicode code-point spans, absent confidence and usage survive unchanged.
3. Repeat without authentication, as a member, and with read-only owner credentials. Expect 401/403 and no provider call.
4. Select the other organization's model. Expect 404 and no provider call.
5. Supply input-level `provider` or `model` overrides, invalid fields, a disabled model or inactive provider. Expect rejection before dispatch.
6. Use a managed provider. Verify it is rejected rather than borrowing hosted credentials or bypassing accounting.
7. Send an oversized body. Verify 413. Return malformed/oversized output or a provider error from the fixture; verify a generic failure without upstream document or credential content.
8. Return a redirect or resolve a registered hostname to a private address. Verify neither destination receives credentials.

## Expected Result

Only authorized tenant BYOK requests dispatch. Unsupported, malformed, oversized and foreign selections fail closed; no deployment utility fallback occurs. Mock qualification does not establish vendor availability or extraction accuracy.
