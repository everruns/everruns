---
type: Test Case
title: "TC003: Admin Login - Sign-out"
description: "Verify that an admin user can successfully sign out after logging in when AUTH_MODE is set to admin."
tags:
  - everruns
  - test-case
  - ui
  - admin-login
---
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
3. Click **Continue with email**
4. Enter password: `1-admin-test`
5. Click **Continue**
6. Verify the user is logged in and on `/chats`
7. Click on the user avatar/menu in the sidebar
8. Click the "Sign out" button

## Expected Result

- User is successfully signed out
- Browser auth cookies are cleared; the current refresh credential is revoked
- User is redirected to the login page
- Attempting to access protected pages redirects to login
- User avatar/menu is no longer visible in the sidebar
