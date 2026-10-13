---
type: Test Case
title: "TC002: Admin Login - Failed Login"
description: "Verify that login fails when invalid credentials are provided in admin authentication mode."
tags:
  - everruns
  - test-case
  - ui
  - admin-login
---
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
| Email variants | Configured admin email, unknown email |
| Password | wrong-password              |

## Steps

1. Navigate to the login page
2. Enter the configured admin email with the wrong password; repeat with `nonexistent@example.com`
3. Click **Continue with email**
4. Enter password: `wrong-password`
5. Click **Continue**

## Expected Result

- Login fails
- User is not authenticated
- Both variants show the same generic credential failure, without revealing account existence
- User remains on the login page
