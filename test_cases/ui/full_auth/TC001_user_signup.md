# TC001: Full Auth - User Sign-up

## Description

Verify that a new user can successfully sign up through the unified
"Log in or sign up" entry when AUTH_MODE is set to full and signup is enabled.
There is no separate registration screen: authenticating with an unknown email
creates the account with the same credentials.

## Preconditions

- `AUTH_MODE=full`
- `AUTH_JWT_SECRET=MJ5SiIlm9mTmiVJV8O2NLrxnuEZDFuO/iXkjVXGqWD0=`
- `AUTH_DISABLE_SIGNUP=false`
- `AUTH_DISABLE_PASSWORD=false`
- No account exists for the test email

## Test Data

| Field    | Value                    |
|----------|--------------------------|
| Email    | testuser@example.com     |
| Password | TestPassword123!         |

## Steps

1. Navigate to the login page (`/login`)
2. Verify the heading reads "Log in or sign up"
3. Enter email: `testuser@example.com`
4. Click "Continue with email"
5. Verify the password screen shows "Continuing as testuser@example.com" and
   mentions the account will be created for a new email
6. Enter password: `TestPassword123!`
7. Click "Continue"

## Expected Result

- Account is created successfully (display name derived from the email)
- User is automatically logged in after signup
- User is redirected to the dashboard/main application
- User session is established with the new account
- User profile shows the correct email
- Navigating to `/register` redirects to `/login` (single unified entry)
