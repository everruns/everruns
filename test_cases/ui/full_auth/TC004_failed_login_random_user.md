# TC004: Full Auth - Failed Login with Random User

## Description

Verify that login fails when attempting to log in with credentials for a non-existent user in full authentication mode.

## Preconditions

- `AUTH_MODE=full`
- `AUTH_JWT_SECRET=MJ5SiIlm9mTmiVJV8O2NLrxnuEZDFuO/iXkjVXGqWD0=`
- `AUTH_DISABLE_SIGNUP=false`
- `AUTH_DISABLE_PASSWORD=false`

## Test Data

| Field    | Value                       |
|----------|-----------------------------|
| Email    | randomuser123@example.com   |
| Password | RandomPassword456!          |

## Steps

1. Navigate to the login page
2. Enter email: `randomuser123@example.com`
3. Enter password: `RandomPassword456!`
4. Click the login button

## Expected Result

- Login fails
- User is not authenticated
- An appropriate error message is displayed (e.g., "Invalid credentials")
- User remains on the login page
- No session is established
