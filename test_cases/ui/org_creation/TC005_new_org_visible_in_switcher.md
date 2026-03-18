# TC005: New Organisation Visible in Sidebar Switcher

## Description

Verify that a newly created organisation appears in the sidebar org dropdown and can be switched to.

## Preconditions

- User is authenticated
- User has at least one existing organisation
- User just created a new organisation (e.g. "Switcher Org")

## Steps

1. Click the organisation dropdown at the top of the sidebar
2. Look for "Switcher Org" in the list
3. Click on "Switcher Org" to switch to it

## Expected Result

- "Switcher Org" appears in the dropdown list
- After clicking, the sidebar shows "Switcher Org" as the current organisation
- Page content reloads to reflect the new org context (e.g. different agents/sessions)
