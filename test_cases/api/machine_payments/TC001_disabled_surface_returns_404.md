# TC001: Disabled machine-payment API surface

## Description

Verify that payment account, policy, and attempt APIs are unavailable when machine payments are disabled.

## Preconditions

- Canonical local stack running with `AUTH_MODE=none`
- `FEATURE_MACHINE_PAYMENTS=false` (or unset)

## Test Data

| Field | Value |
|-------|-------|
| Account ID | A valid, nonexistent UUID |
| Policy ID | A valid, nonexistent UUID |

## Steps

1. Request `GET /v1/feature-flags` and record `machine_payments`.
2. Call `GET` and `POST` on `/v1/payments/accounts`.
3. Call `GET`, `PATCH`, and `DELETE` on `/v1/payments/accounts/{account_id}`.
4. Call `GET` and `POST` on `/v1/payments/policies`.
5. Call `GET`, `PATCH`, and `DELETE` on `/v1/payments/policies/{payment_policy_id}`.
6. Call `GET /v1/payments/attempts`.

## Expected Result

- The feature-flags response contains `"machine_payments": false`.
- Every payment endpoint returns HTTP 404, including syntactically valid create requests.
- No payment account, policy, or attempt record is created.
