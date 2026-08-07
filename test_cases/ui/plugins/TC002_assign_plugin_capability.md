# Assign an installed plugin capability

## Description

Verify that an installed plugin can be assigned to an agent with stable identity and a valid icon,
and that stale plugin assignments remain visible and removable.

## Preconditions

- Everruns is running in development mode with authentication disabled.
- The default Everruns marketplace is present and synced.
- The Resend plugin is available in the marketplace.

## Test Data

| Field | Value |
|---|---|
| Plugin | Resend |
| Agent | Dad Jokes |

## Steps

1. Open `/plugins`, browse the default marketplace, and install Resend.
2. Open the Dad Jokes agent edit page and add Resend from the capability selector.
3. Confirm Resend uses its manifest icon or the neutral plugin fallback, not a disabled symbol.
4. Save the agent, reload the edit page, and confirm Resend remains attached.
5. Start a new session for Dad Jokes and confirm the Resend MCP tools are present after the required
   connection is configured.
6. Disable or uninstall Resend, return to the open editor, and confirm the stale assignment is shown
   as an inline actionable error without clearing other edits.
7. Remove the stale assignment and save the remaining changes.

## Expected Result

- Install, selection, save, reload, and session tool loading use one stable capability reference.
- Plugin updates retain assignments; uninstall/reinstall does not silently rebind old assignments.
- Missing, malformed, or remote icon metadata uses a neutral local fallback.
- Stale assignments are visible and removable, and unrelated unsaved form changes remain intact.
