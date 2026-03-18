# TC001: Admin Login - Success Login

## Description

Verify that an admin user can successfully log in when AUTH_MODE is set to admin and valid credentials are provided.

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

## Expected Result

- Login succeeds
- User is authenticated and redirected to the main application
- User session is established
