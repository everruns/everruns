# TC001: Full Auth - User Sign-up

## Description

Verify that a new user can successfully sign up when AUTH_MODE is set to full and signup is enabled.

## Preconditions

- `AUTH_MODE=full`
- `AUTH_JWT_SECRET=MJ5SiIlm9mTmiVJV8O2NLrxnuEZDFuO/iXkjVXGqWD0=`
- `AUTH_DISABLE_SIGNUP=false`
- `AUTH_DISABLE_PASSWORD=false`

## Test Data

| Field    | Value                    |
|----------|--------------------------|
| Name     | Test User                |
| Email    | testuser@example.com     |
| Password | TestPassword123!         |

## Steps

1. Navigate to the login page
2. Click the "Sign up" or "Create account" link
3. Enter name: `Test User`
4. Enter email: `testuser@example.com`
5. Enter password: `TestPassword123!`
6. Click the "Create account" button

## Expected Result

- Account is created successfully
- User is automatically logged in after signup
- User is redirected to the dashboard/main application
- User session is established with the new account
- User profile shows the correct name and email
