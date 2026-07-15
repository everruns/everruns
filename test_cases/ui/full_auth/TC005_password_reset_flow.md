# TC005: Full Auth - Password Reset Flow

## Description

Verify the full self-service password reset: request link, set a new
password, old sessions revoked, expired/invalid links handled gracefully.

## Preconditions

- `AUTH_MODE=full`, `AUTH_DISABLE_PASSWORD=false`
- Email delivery configured (`EMAIL_PROVIDER`), or read the reset link from
  server logs/dev mailbox
- Account exists for `testuser@example.com` (TC001)

## Steps

1. Navigate to `/login` → enter email → "Continue with email" → click
   "Forgot password?"
2. Verify the branded two-panel page ("Reset your password"), enter
   `testuser@example.com`, click "Send reset link"
3. Verify the enumeration-safe confirmation ("If an account exists for …")
4. Open the emailed `/reset-password?token=…` link
5. Enter mismatched passwords → inline alert "Passwords do not match"
6. Enter a valid new password twice → "Update password"
7. Verify the success state and "Continue to sign in"
8. Sign in with the NEW password → succeeds; old password → generic error
9. Re-open the same reset link → "Invalid reset link" state with
   "Request a new link" (token is single-use)
10. Request a link, wait > 1 hour (or use an expired token) → same
    "Invalid reset link" state

## Expected Result

- All states render on the branded AuthShell with an h1 heading and
  `role="alert"` errors
- No response ever reveals whether an account exists
- After reset, previously issued sessions are signed out (refresh revoked)
