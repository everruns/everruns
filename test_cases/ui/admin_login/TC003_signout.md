# TC003: Admin Login - Sign-out

## Description

Verify that an admin user can successfully sign out after logging in when AUTH_MODE is set to admin.

## Preconditions

- `AUTH_MODE=admin`
- `AUTH_ADMIN_EMAIL=test-admin-1@example.com`
- `AUTH_ADMIN_PASSWORD=1-admin-test`

## Test Data

| Field    | Value                      |
|----------|----------------------------|
| Email    | test-admin-1@example.com   |
| Password | 1-admin-test               |

## Steps

1. Navigate to the login page
2. Enter email: `test-admin-1@example.com`
3. Enter password: `1-admin-test`
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
