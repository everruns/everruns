---
type: Test Case
title: "TC004: Auth - Deployment Modes"
description: "Verify none, admin, full, and external deployments expose the intended sign-in paths and reject unsupported authentication configuration."
tags:
  - everruns
  - test-case
  - api
  - auth
---
# TC004: Auth - Deployment Modes

## Description

Verify none, admin, full, and external deployments expose the intended sign-in paths and reject unsupported authentication configuration.

## Preconditions

- Isolated disposable deployments using the canonical startup contract and a distinct port prefix each.
- Matching server/UI auth mode; private test secrets configured through the environment.
- Admin credentials, a verified full-auth account, and a working external backend fixture.
- Startup-negative checks use a short-lived config-validation process with a 10-second deadline; do not restart a shared deployment.

## Test Data

| Mode | Browser behavior |
| --- | --- |
| none, deployment grade dev | `/login` proceeds to Chats; anonymous default-org account |
| admin | Two-step password door; configured admin can sign in; signup unavailable |
| full | Password/OAuth/signup affordances follow enabled configuration |
| external | Built-in password/signup/OAuth unavailable; configured provider owns sign-in |
| none, non-dev deployment grade | Startup fails |
| Unknown AUTH_MODE | Startup fails; no anonymous fallback |

## Steps

1. Verify `/health` and `/api/v1/auth/config` for each supported fixture; record the actual mode and configured flags without secrets.
2. In a clean browser without access/refresh cookies, open `/agents`, then the appropriate login path. Complete sign-in for each authenticated fixture.
3. Compare visible signup/password/OAuth affordances against the config, and try disabled signup/password endpoints with disposable inputs; they must not authenticate or create an account.
4. For external mode, verify login delegation and the configured backend credential, then repeat with an invalid credential.
5. Exercise the two startup-negative variants in bounded config-validation processes and record their exit status.

## Expected Result

- Browser and API agree on mode. Authenticated deployments require valid credentials; invalid ones cannot fall back to anonymous access.
- Admin and external deployments do not offer built-in signup. Full mode follows its signup/password/provider configuration.
- Local no-auth uses the anonymous default-org account and does not strand the user at a blank login screen.
- Non-dev no-auth and unknown modes fail configuration validation before serving protected resources.

## Source Contract

[Auth configuration](../../../../crates/server/src/auth/config.rs) and [authentication modes](../../../security/authentication.md).
