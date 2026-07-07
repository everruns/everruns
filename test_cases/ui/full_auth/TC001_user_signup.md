# TC001: Full Auth - User Sign-up

## Description

Verify that a new user can sign up through the explicit "Create an account"
path when AUTH_MODE is full and signup is enabled. Login and signup are
separate screens; the login door links to `/signup`. With
`AUTH_SIGNUP_EMAIL_CONFIRM=true` (SaaS/dev), signup always ends at a
"Check your email" landing and the emailed confirmation link both verifies
the address and signs the user in.

## Preconditions

- `AUTH_MODE=full`
- `AUTH_JWT_SECRET=MJ5SiIlm9mTmiVJV8O2NLrxnuEZDFuO/iXkjVXGqWD0=`
- `AUTH_DISABLE_SIGNUP=false`, `AUTH_DISABLE_PASSWORD=false`
- Confirm mode: `AUTH_SIGNUP_EMAIL_CONFIRM=true` + email delivery configured
- No account exists for the test email

## Test Data

| Field    | Value                    |
|----------|--------------------------|
| Email    | testuser@example.com     |
| Password | TestPassword123!         |

## Steps

1. Navigate to `/login`; verify the heading reads "Log in" and the subline
   offers "New to Everruns? Create an account"
2. Click "Create an account" → lands on `/signup` ("Create your account",
   SSO buttons first, then email + password)
3. Type a short password (e.g. `abc1`) — verify the inline requirements
   ("At least 12 characters", "At least one number") stay unchecked and
   submit is blocked with a calm inline message
4. Enter password `TestPassword123!` — both requirement rows tick
5. Click "Create account"
6. Verify the "Check your email" landing: copy reads "If
   testuser@example.com can be registered, we've sent a confirmation link"
   (identical whether or not the address already exists), with
   "Use a different email" and "Log in" links
7. Open the emailed confirmation link (`/verify-email?token=…`)
8. Verify the email-verified state and that a session now exists — Continue
   lands in the onboarding arc (organisation step), not back at login

## Expected Result

- Account is created unverified at step 5; NO session until step 7
- Repeating steps 2–5 with an already-registered email shows the exact same
  on-screen outcome; that address receives a "you already have an account"
  email instead
- Confirmation link is single-use; reusing it shows the generic failure state
- Self-host mode (`AUTH_SIGNUP_EMAIL_CONFIRM` unset): step 5 instead creates
  the account AND signs in immediately (no check-email interstitial)
