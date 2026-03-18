# TC002: Create Organisation from Sidebar Dropdown

## Description

Verify that a user can create a new organisation via the sidebar org dropdown.

## Preconditions

- User is authenticated

## Steps

1. Click the organisation dropdown at the top of the sidebar
2. Click "Create Organisation" at the bottom of the dropdown
3. In the dialog, enter name: `Sidebar Org`
4. Click "Create"

## Expected Result

- Dialog opens with name input and Create/Cancel buttons
- On success, user is redirected to the setup page for the new org
- The sidebar dropdown now shows "Sidebar Org" as the current organisation
