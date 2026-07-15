# TC008: Full Auth - Reachability Dead-End & Trap Recovery

## Description

Walk the scenarios the reachability model (`apps/ui/src/lib/auth-flow/machine.ts`)
exists to protect: situations where a screen historically offered a way forward
that silently no-ops or dead-ends for a particular hidden account state. Each
must now surface a path that actually works. These are the manual counterpart to
the `machine.test.ts` invariants (structural reachability + no misleading
remediation).

Account states: **S1** `local_unverified`, **S2** `local_verified`,
**S3** `oauth_only`.

## Preconditions

- `AUTH_MODE=full`; email delivery configured (`EMAIL_PROVIDER`) or read links
  from the dev mailbox/server logs
- Google and/or GitHub OAuth configured (scenarios A, B)
- Accounts available:
  - an **S3 oauth_only** account (created via "Continue with Google", never given
    a password)
  - an **S2 local_verified** account bound to a password
  - an **S1 local_unverified** account with an org membership (accept an invite,
    or let verification lapse) for scenario C

## Steps

### Scenario A — oauth_only user typed a password, then reached for reset (closed trap #1)

1. `/login` → enter the **S3 oauth_only** account's email → "Continue with email".
2. On `login.password`, type any password → "Continue". Verify the submit
   **fails** (no usable password on this account).
3. Verify the credential-failure alert names the **OAuth alternative** — copy
   shown to everyone, e.g. "Signed up with Google or GitHub? Go back and use
   that instead" — and does **not** present password reset as the only way out.
   (A reset request for an oauth_only account produces no email, so offering
   only reset would strand the user.)
4. Follow it: "Back" → `login.email` → "Continue with Google" → **authenticated**.

### Scenario B — permanent OAuth link-refusal (closed trap #2)

1. Ensure the target verified email already has an account bound to a different
   method (e.g. a local password account for that address).
2. `/login` → "Continue with <provider>" and complete consent with that email so
   the server refuses to link.
3. Verify the outcome is the **permanent** category: `409 → oauth_account_exists`,
   landing on the login door with copy that **names the way through** ("sign in
   with your original method"), not the transient "didn't complete, try again".
4. Sign in via the original method from the same door → **authenticated**.

### Scenario C — signed-in but unverified, blocked by a verification gate (app.gated_on_verify)

1. Sign in as the **S1 local_unverified** account that already belongs to an org
   (so onboarding's zero-org verify gate does not apply).
2. Trigger a verification-gated action (e.g. accept an org invite / hit
   `TM-AUTH-023`). Verify it is blocked pending verification.
3. Verify a **persistent verify-email banner** is present on the authenticated
   screens with a working **Resend**. Click it.
4. Click the emailed link → email verified → retry the gated action → it now
   **succeeds**.

### Scenario D — expired verification link with no email param (verify.failed_no_email)

1. Open `/verify-email?token=garbage` (no `email=` param).
2. Verify the failed state does **not** dead-end on "sign in to request a new
   verification email". It accepts an email and **resends in place**.
3. Enter the S1 account's address → resend → a fresh verification email arrives →
   click it → **email verified**.

## Expected Result

- Every scenario reaches its goal (authenticated / email_verified) via an
  affordance that genuinely works for that account state — no silent no-op, no
  dead end, no raw JSON page.
- Recovery copy stays enumeration-safe: generic copy shown to everyone
  (scenario A alt), an action on the user's own authenticated account
  (scenario C banner), or addressed to a caller who already proved mailbox
  ownership (scenario B `oauth_account_exists`).
- `machine.test.ts` remains green — `findDeadEnds()` and `findTraps()` both empty.
