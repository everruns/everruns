# TC002: Enabled machine-payment custody surface

## Description

Verify that payment navigation and wallet/policy management remain available when machine payments are enabled.

## Preconditions

- Canonical local stack running with `AUTH_MODE=none`
- `FEATURE_MACHINE_PAYMENTS=true`
- Browser session open as the local organization owner

## Test Data

| Field | Value |
|-------|-------|
| Wallet key | Newly generated disposable local-only Base test key |
| Wallet label | `Machine payment UI smoke test` |

## Steps

1. Open Settings and select Payments.
2. Verify global search finds Settings > Payments.
3. Create an organization-owned x402/Base payment account with the disposable key.
4. Create a spend policy for that account.
5. Refresh the page and inspect the account, policy, and attempts sections.
6. Disable the policy and account.

## Expected Result

- The Payments link and page are visible.
- Global search links to `/settings/payments`.
- The account and policy can be created and disabled.
- The private key is not displayed after submission or refresh.
- The attempts section loads without a feature-disabled error.
