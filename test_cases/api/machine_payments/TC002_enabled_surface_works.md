# TC002: Enabled machine-payment API surface

## Description

Verify that payment account, policy, and attempt APIs retain their working behavior when machine payments are enabled.

## Preconditions

- Canonical local stack running with `AUTH_MODE=none`
- `FEATURE_MACHINE_PAYMENTS=true`
- Stable local-development encryption key configured by the startup contract

## Test Data

| Field | Value |
|-------|-------|
| Wallet key | Newly generated disposable local-only Base test key |
| Wallet label | `Machine payment smoke test` |
| Allowed host | `parallelmpp.dev` |
| Per-request limit | `0.01` USD |

## Steps

1. Request `GET /v1/feature-flags` and record `machine_payments`.
2. Create an organization-owned x402/Base payment account with the disposable key.
3. List and fetch the account; verify no private key is returned.
4. Create a policy for the account and list/fetch it.
5. List payment attempts.
6. Disable the policy and account.

## Expected Result

- The feature-flags response contains `"machine_payments": true`.
- Account and policy create/list/get/disable operations return their documented success statuses.
- Account responses never contain the submitted private key.
- The attempts endpoint returns an array.
