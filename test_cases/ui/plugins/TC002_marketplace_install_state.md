# Marketplace plugin install state

## Description

Verify that installing a marketplace plugin reconciles the catalog and installed-plugin views
without a reload or duplicate install request.

## Preconditions

- Everruns is running in development mode with authentication disabled.
- The default Everruns marketplace is present and synced.
- Resend is not installed.

## Test Data

| Field | Value |
|---|---|
| Route | `/plugins` |
| Marketplace | `everruns` |
| Plugin | `resend` |

## Steps

1. Open `/plugins`, select **Marketplaces**, and browse the `everruns` catalog.
2. Select **Install** for Resend and confirm the action immediately becomes disabled and reads
   **Installing...**.
3. Wait for installation to finish without closing the catalog.
4. Confirm the Resend row reads **Installed**, its action is disabled, and no error is shown.
5. Close the catalog and select **Installed Plugins**.
6. Confirm Resend is present without reloading the page.
7. Return to the marketplace catalog and confirm Resend still reads **Installed**.

## Expected Result

- Only one install request can be initiated while installation is pending.
- The open catalog and installed-plugin inventory reconcile after success.
- Installed state survives closing and reopening the catalog.
- No rejected-promise runtime overlay or console error appears.
