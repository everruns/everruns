# TC004: Verify Harnesses Exist After Org Setup

## Description

Verify that built-in harnesses are provisioned for the new organisation by navigating to the Harnesses page after setup. Harnesses now use addressable names (slugs) with separate display names.

## Preconditions

- User just completed org setup (all three steps show checkmarks)

## Steps

1. On the setup page, click "Go to dashboard"
2. Navigate to the Harnesses page via the sidebar
3. Review the list of harnesses displayed

## Expected Result

- Dashboard loads without errors
- Harnesses page shows at least three built-in harnesses with names: `base`, `generic`, `platform-chat`
- Each harness card shows a "Built-in" badge
- Each harness card is clickable and links to a detail page
