# TC007: Full Auth - OAuth Sign-in Error Paths

## Description

Verify OAuth (Google/GitHub) failure handling: the callback never dead-ends
on raw JSON; the login door shows friendly copy per category.

## Preconditions

- `AUTH_MODE=full` with Google and/or GitHub OAuth configured

## Steps

1. `/login` → "Continue with Google" → cancel on the consent screen
2. Verify redirect back to `/login?error=oauth_cancelled` with the alert
   "Sign-in was cancelled. You can try again anytime."
3. With `AUTH_GOOGLE_ALLOWED_DOMAINS` set, sign in with an out-of-domain
   account → `/login?error=oauth_not_permitted` with the "can't be used to
   sign in here" alert
4. Simulate a broken exchange (revoke client secret in a dev env) →
   `/login?error=oauth_failed` with the generic retry copy
5. Open `/login?error=<script>x</script>` → generic copy renders (unknown
   categories fall back; no injection)
6. Double-click an SSO button → it disables after the first click

## Expected Result

- Every OAuth failure lands on the branded login door, never a JSON page
- Copy is coarse and friendly; details only in server logs/audit trail
- Same-email verified-provider sign-in still links to an existing password
  account and logs in (see OSS #2570)
