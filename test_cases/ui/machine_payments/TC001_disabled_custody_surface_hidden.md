# TC001: Disabled machine-payment custody surface

## Description

Verify that a deployment with machine payments disabled does not advertise or render wallet custody controls.

## Preconditions

- Canonical local stack running with `AUTH_MODE=none`
- `FEATURE_MACHINE_PAYMENTS=false` (or unset)
- Browser session open as the local organization owner

## Test Data

None.

## Steps

1. Open `/settings/organization`.
2. Inspect the Settings navigation for Payments.
3. Open global search and search for `payments` and `wallet`.
4. Navigate directly to `/settings/payments`.
5. Inspect browser network activity for `/v1/payments/` requests.

## Expected Result

- Settings navigation contains no Payments link.
- Global search contains no `/settings/payments` result.
- Direct navigation renders the application 404 state.
- No wallet/private-key form is rendered.
- No payment account, policy, or attempt request is sent.
