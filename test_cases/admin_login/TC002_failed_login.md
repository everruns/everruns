# TC002: Admin Login - Failed Login

## Description

Verify that login fails when invalid credentials are provided in admin authentication mode.

## Preconditions

- `AUTH_MODE=admin`
- `AUTH_ADMIN_EMAIL=test-admin-1@example.com`
- `AUTH_ADMIN_PASSWORD=1-admin-test`

## Test Data

| Field    | Value                       |
|----------|-----------------------------|
| Email    | nonexistent@example.com     |
| Password | wrong-password              |

## Steps

1. Navigate to the login page
2. Enter email: `nonexistent@example.com`
3. Enter password: `wrong-password`
4. Click the login button

## Expected Result

- Login fails
- User is not authenticated
- An appropriate error message is displayed (e.g., "Invalid credentials")
- User remains on the login page
