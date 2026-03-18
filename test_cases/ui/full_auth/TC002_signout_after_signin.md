# TC002: Full Auth - Sign-out After Sign-in

## Description

Verify that a user can successfully sign out after signing in when AUTH_MODE is set to full.

## Preconditions

- `AUTH_MODE=full`
- `AUTH_JWT_SECRET=MJ5SiIlm9mTmiVJV8O2NLrxnuEZDFuO/iXkjVXGqWD0=`
- `AUTH_DISABLE_SIGNUP=false`
- `AUTH_DISABLE_PASSWORD=false`
- A user account exists (created via TC001 or pre-existing)

## Test Data

| Field    | Value                    |
|----------|--------------------------|
| Email    | testuser@example.com     |
| Password | TestPassword123!         |

## Steps

1. Navigate to the login page
2. Enter email: `testuser@example.com`
3. Enter password: `TestPassword123!`
4. Click the login button
5. Verify user is logged in and on the dashboard
6. Click on the user avatar/menu in the sidebar
7. Click the "Sign out" button

## Expected Result

- User is successfully signed out
- User session is terminated
- User is redirected to the login page
- Attempting to access protected pages redirects to login
- User avatar/menu is no longer visible in the sidebar
