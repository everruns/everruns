# TC001: Create Organisation from Settings Page

## Description

Verify that a user can create a new organisation via the Settings > Organisation page and is redirected to the setup page.

## Preconditions

- User is authenticated
- User has permission to create organisations

## Steps

1. Navigate to Settings > Organisation
2. Click the "Create Organisation" button (top right of "Your Organisations" section)
3. In the dialog, enter name: `Test Org Alpha`
4. Click "Create"

## Expected Result

- Dialog shows "Creating..." while the request is in progress
- On success, user is redirected to `/orgs/<orgId>/setup`
- The setup page shows the org name "Test Org Alpha" in the header
