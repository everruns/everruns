# TC006: Full Auth - Email Verification Flow

## Description

Verify email verification: pending state, resend with cooldown, wrong-address
recovery, token consumption, invalid/expired token handling.

## Preconditions

- `AUTH_MODE=full`, signup enabled, email delivery configured
- Fresh signup for `verifyme@example.com` (TC001 flow)

## Steps

1. Open `/verify-email?email=verifyme%40example.com` (no token)
2. Verify the pending state: "Verify your email", the address shown, a
   "Resend verification email" button, and "Wrong address? Use a different
   email" linking to `/login`
3. Click resend → button locks with a countdown ("Resend link · 0:59…")
4. Click the emailed verification link → "Your email is verified" →
   "Continue"
5. Re-open the same link → "Verification failed" (single-use token)
6. Open `/verify-email?token=garbage` → "Verification failed" with resend
   guidance
7. Repeat resend twice within a minute (API level) → second send is silently
   skipped (server budget); UI cooldown mirrors it

## Expected Result

- All states on the branded AuthShell, h1 headings, generic failure copy
- Resend is enumeration-safe and rate-limited (1/min, small daily cap)
- Verified flag unblocks the SaaS org-creation gate on next refresh
