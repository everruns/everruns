# TC004: Full Auth - Failed Login (Wrong Password / Unknown User)

## Description

Verify that authentication fails with a generic message when the password is
wrong, and that the unified "Log in or sign up" door never reveals whether an
account exists for an email.

Note: with signup enabled, entering an unknown email with a valid (8+ char)
password intentionally **creates** the account (unified door — see TC001).
A failed outcome for an unknown user therefore requires either a wrong
password for an existing account, or signup disabled.

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
- A generic error message is displayed (e.g. "Invalid email or password.") —
  it must not reveal whether the account exists or which field was wrong
- User remains on the password screen and can retry or go back

## Variant: signup disabled

With `AUTH_DISABLE_SIGNUP=true`, repeat with an unknown email
(`randomuser123@example.com` / any password): the heading reads "Welcome
back", authentication fails with the same generic message, and no account is
created.
