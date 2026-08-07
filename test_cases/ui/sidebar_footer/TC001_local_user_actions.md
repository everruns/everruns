# TC001: Local User Sidebar Actions

## Description

Verify the sidebar footer exposes actions supported by the current server capabilities in no-auth
mode without presenting the local user as authenticated, and preserves the authenticated menu.

## Preconditions

- Everruns stack available in `AUTH_MODE=none`
- A second Everruns stack available with authentication enabled
- `mcp_endpoint` enabled for the first pass and disabled for the capability-gating pass

## Test Data

| Field | Value |
| --- | --- |
| Local user | Server-provided anonymous user |
| Viewports | Desktop and mobile |

## Steps

1. Open the no-auth stack on desktop and inspect the sidebar footer.
2. Open the local identity menu with a pointer, then close and reopen it with the keyboard.
3. Verify Profile is present and Personal access tokens and Sign out are absent.
4. Open Connect via MCP and verify the server URL, configuration snippet, copy controls, and
   no-auth guidance are visible.
5. Open the mobile navigation drawer and repeat steps 2–4.
6. Disable `mcp_endpoint`, reload, and verify Connect via MCP is absent while Profile remains.
7. Open the authenticated stack, sign in, and inspect the identity menu on desktop and mobile.
8. Verify Profile, Personal access tokens, Connect via MCP, and Sign out are present and the MCP
   dialog retains its OAuth guidance.

## Expected Result

- The local footer identifies the server-provided user as a local user and displays the version.
- Profile and feature-enabled MCP connection details are reachable in both desktop and mobile UI.
- Authentication-only Personal access tokens and Sign out actions are absent in no-auth mode.
- MCP Connect follows the `mcp_endpoint` capability in either auth mode.
- The authenticated menu and logout behavior are unchanged.
- Menu and dialog controls are keyboard-operable, labeled, and remain within the mobile viewport.
