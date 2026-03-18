# TC006: Organisation Settings Populated After Creation

## Description

Verify that Settings > Organisation shows correct details for a newly created org, including default and base harness assignments.

## Preconditions

- User created a new organisation and completed setup
- User is currently viewing the new org

## Steps

1. Navigate to Settings > Organisation
2. Review the "Current Organisation" section
3. Check the Default Harness and Base Harness dropdowns
4. Check the Organisation ID field

## Expected Result

- Organisation name matches the name entered during creation
- Default Harness dropdown has a harness selected (not empty)
- Base Harness dropdown has a harness selected (not empty)
- Organisation ID is displayed and starts with `org_`
- A copy button is present next to the Organisation ID
