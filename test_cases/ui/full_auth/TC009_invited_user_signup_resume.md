# TC009 — Invited user signs up and accepts invitation

## Description

Verifies that an organization invite link preserves its target through the
login → signup → email verification path and resumes invitation acceptance for a
brand-new user.

## Preconditions

- `AUTH_MODE=full`, signup enabled, password auth enabled
- `AUTH_SIGNUP_EMAIL_CONFIRM=true` with working email delivery (or AgentMail in dev)
- An org owner account can send invitations
- Clean browser session (no existing Everruns cookies)

## Test Data

| Field | Value |
| --- | --- |
| Invitee email | `everruns-testing+<unique>@agentmail.to` |
| Password | `ValidPass123` (12+ chars with a number) |

## Steps

1. Sign in as an org owner and invite the invitee email.
2. Open the emailed `/invite/<token>` link in the clean browser.
3. Confirm redirect to `/login?return_to=%2Finvite%2F<token>`.
4. Click **Create an account** on the login page.
5. Confirm the signup URL includes `return_to=%2Finvite%2F<token>`.
6. Complete signup with the invitee email and password.
7. Open the verification link from email.
8. Click **Continue** on the verified-email screen.

## Expected Result

- Step 4–5: signup retains the sanitized invite `return_to`.
- Step 8: the browser resumes `/invite/<token>` (not plain `/onboarding`).
- Invitation acceptance succeeds and the invitee lands in the invited org.
