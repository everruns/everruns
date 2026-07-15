# TC004: Full Auth - Failed Login (Wrong Password / Unknown User)

## Description

Verify that authentication fails with a calm generic message when the
password is wrong or the account is unknown, and that the login door never
reveals whether an account exists for an email. Login never creates
accounts — signup is the explicit `/signup` path (TC001).

## Preconditions

- `AUTH_MODE=full`
- `AUTH_JWT_SECRET=MJ5SiIlm9mTmiVJV8O2NLrxnuEZDFuO/iXkjVXGqWD0=`
- `AUTH_DISABLE_PASSWORD=false`
- An account exists for `testuser@example.com` (created via TC001)

## Test Data

| Field    | Value                       |
|----------|-----------------------------|
| Email    | testuser@example.com        |
| Password | WrongPassword456!           |

## Steps

1. Navigate to the login page (`/login`)
2. Enter email: `testuser@example.com`
3. Click "Continue with email"
4. Enter password: `WrongPassword456!`
5. Click "Continue"

## Expected Result

- Authentication fails; no session is established
- A calm, muted (not alarm-red) alert reads "Email or password doesn't
  match. Try again or reset your password." — with "reset your password"
  linking to `/forgot-password` prefilled with the email. It must not reveal
  whether the account exists or which field was wrong
- User remains on the password screen and can retry or go back

## Variant: unknown email

Repeat with `randomuser123@example.com` / any password: identical generic
alert, no account created (login never signs up — see TC001 for the
explicit signup path).

## Variant: signup disabled

With `AUTH_DISABLE_SIGNUP=true`: the "Create an account" link disappears
from `/login` and `/signup` shows "Signups are closed"; failed logins behave
exactly as above.
