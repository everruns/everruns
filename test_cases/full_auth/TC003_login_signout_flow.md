# TC003: Full Auth - Login with Signed Up User, Sign-out

## Description

Verify the complete flow of logging in with a previously signed up user and then signing out.

## Preconditions

- `AUTH_MODE=full`
- `AUTH_JWT_SECRET=MJ5SiIlm9mTmiVJV8O2NLrxnuEZDFuO/iXkjVXGqWD0=`
- `AUTH_DISABLE_SIGNUP=false`
- `AUTH_DISABLE_PASSWORD=false`
- User has previously signed up (TC001 completed)

## Test Data

| Field    | Value                    |
|----------|--------------------------|
| Email    | testuser@example.com     |
| Password | TestPassword123!         |

## Steps

1. Navigate to the login page (ensure not already logged in)
2. Enter email: `testuser@example.com`
3. Enter password: `TestPassword123!`
4. Click the login button
5. Verify successful login:
   - User is redirected to dashboard
   - User name and email are displayed in sidebar
6. Navigate to different pages (Dashboard, Agents, Capabilities)
7. Return to Dashboard
8. Click on the user avatar/menu in the sidebar
9. Click the "Sign out" button
10. Verify successful sign-out:
    - User is redirected to login page
    - Session is cleared

## Expected Result

- Login succeeds with the signed up user credentials
- User can navigate freely within the application while logged in
- Sign-out succeeds and terminates the session
- After sign-out, user cannot access protected pages without re-authenticating
