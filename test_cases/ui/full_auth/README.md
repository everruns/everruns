# Full Auth — State-Machine Coverage Map

These cases exercise the auth flow whose reachability contract is modelled in
`apps/ui/src/lib/auth-flow/machine.ts` and enforced by `machine.test.ts`
(structural reachability + no-misleading-remediation, ratcheted in CI). The
diagram of that model lives in `specs/authentication.md` § Flow Reachability.

`machine.test.ts` is the **automated** guard on the model itself; the cases
below are the **manual** walk-throughs of the same flows against a running
`AUTH_MODE=full` stack. Keep them aligned: when an auth screen's affordances
change, update `machine.ts`, keep `machine.test.ts` green, and update the case
here that walks that path.

| State-machine scenario (machine.ts) | Case |
|---|---|
| Brand-new user creates an account → verify link → authenticated | `TC001_user_signup.md`, `TC006_email_verification_flow.md` |
| Verified user signs in / signs out | `TC002_signout_after_signin.md`, `TC003_login_signout_flow.md` |
| Failed login (wrong/unknown credentials) | `TC004_failed_login_random_user.md` |
| Verified user who forgot their password recovers | `TC005_password_reset_flow.md` |
| Unverified user verifies (pending / resend / wrong-address / expired) | `TC006_email_verification_flow.md` |
| OAuth rejection categories land on the login door, never a dead end | `TC007_oauth_error_paths.md` |
| **Closed traps + gated-verify recovery** (no path silently no-ops) | `TC008_reachability_dead_end_recovery.md` |
| Protected route delegates login to a trusted remote origin | `TC010_external_login_origin.md` |

Account states referenced across the suite (see `machine.ts`):
**S0** `none` · **S1** `local_unverified` · **S2** `local_verified` ·
**S3** `oauth_only`.
