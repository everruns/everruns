# TC010: Full Auth - External Login Origin

## Description

Verify that protected routes can delegate the login page to a trusted remote
origin while preserving a safe relative `return_to` continuation.

## Preconditions

- `AUTH_MODE=full`
- `AUTH_LOGIN_ORIGIN=https://id.example.com` is set identically for the server
  and UI runtimes
- Clean browser session with no `access_token` cookie

## Steps

1. Navigate to `/settings/providers?tab=models` on the app origin.
2. Observe the browser navigation target.
3. Repeat with an OAuth-style protected continuation:
   `/oauth/authorize?client_id=test&response_type=code`.
4. Inspect `GET /v1/auth/config` from the app API.
5. Remove `AUTH_LOGIN_ORIGIN`, restart the stack, and repeat step 1.

## Expected Result

- Steps 1–2 use a full-page navigation to
  `https://id.example.com/login?return_to=%2Fsettings%2Fproviders%3Ftab%3Dmodels`;
  the Next.js client router is not used.
- Step 3 preserves the entire relative OAuth continuation in `return_to`.
- Step 4 returns `login_origin: "https://id.example.com"` and exposes no way
  for request or query input to override it.
- Step 5 preserves the original relative same-origin redirect byte-for-byte:
  `/login?return_to=%2Fsettings%2Fproviders%3Ftab%3Dmodels`.
- Absolute, protocol-relative, or backslash-prefixed `return_to` values remain
  rejected by the login page sanitizer.
