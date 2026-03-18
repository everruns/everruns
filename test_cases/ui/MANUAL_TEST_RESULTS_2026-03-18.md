# Manual UI Test Results - 2026-03-18

## Environment

- **Auth Mode**: Full (signup + password enabled)
- **Stack**: PostgreSQL + Valkey + API + Worker + UI + Caddy
- **PORT_PREFIX**: 271 (proxy: 27100, API: 27101, UI: 27105, etc.)
- **Browser**: Chromium 141 (headless, via agent-browser)

## Test Summary

| Category | Tests | Pass | Fail/Partial | Issues |
|----------|-------|------|-------------|--------|
| Full Auth | 4 | 4 | 0 | 0 |
| Organisation Creation | 6 | 2 | 4 | 4 |
| MCP Servers | 5 | 4 | 1 | 2 |
| Global Chat | 1 | 1 | 0 | 1 |
| Command Palette | 1 | 1 | 0 | 0 |
| Scheduled Tasks | 1 | 0 | 1 | 2 |
| **Total** | **18** | **12** | **6** | **9** |

## Detailed Results

### Full Auth (4/4 PASS)

- **TC001 User Signup**: PASS - Account created, redirected to dashboard, profile shows correct name/email
- **TC002 Signout After Signin**: PASS - Signed out, redirected to login, protected pages redirect to login
- **TC003 Login/Navigate/Signout Flow**: PASS - Login succeeds, navigation between Dashboard/Agents/Capabilities works, signout works
- **TC004 Failed Login Random User**: PASS - Error "Invalid email or password" displayed, stays on login page

### Organisation Creation (2/6 PASS)

- **TC001 Create Org from Settings**: PARTIAL FAIL - "Create Organisation" button at bottom of Settings > General page doesn't open dialog on first click (see Issue #3)
- **TC002 Create Org from Sidebar**: PASS - Dialog opens, name entered, org created
- **TC003 Setup Page Progress**: FAIL - Not redirected to setup page after creation (see Issue #3)
- **TC004 Verify Harnesses After Setup**: SKIP - Depends on TC003
- **TC005 New Org Visible in Switcher**: PARTIAL PASS - Org appears after page reload, but not immediately after creation
- **TC006 Org Settings After Creation**: PARTIAL FAIL - Harness dropdowns show raw IDs instead of names (see Issue #4)

### MCP Servers (4/5 PASS)

- **TC001 Create MCP Server**: PASS - Server created with name, description, URL
- **TC004 List MCP Servers**: PASS - Server card shows all details correctly
- **TC007 Archive MCP Server**: PARTIAL PASS - Archive works but no confirmation dialog (see Issue #9)
- **TC008 Validation Empty Name**: PASS - Create button stays disabled
- **TC009 Validation Empty URL**: PASS (browser validation) - Button enables without URL (minor UX issue), but browser `required` attribute prevents submission

### Global Chat (1/1 PASS)

- **TC001 Global Chat Loads**: PASS - Chat page loads with empty state, input field, model selector

### Command Palette (1/1 PASS)

- **TC004 Command Palette UI**: PASS - Opens via click on search button, searches pages/agents/capabilities, navigates correctly

### Scheduled Tasks (0/1 PASS)

- **TC001 Create Schedule Basic**: FAIL - Schedule creation from UI dialog silently fails; schedule does not appear in list (see Issue #12)

## Issues Found

### Issue #1 (Low/Dev-mode): Sidebar navigation shows stale page during Next.js compilation
- **Severity**: Low (dev-mode only)
- **Steps**: Click sidebar link (e.g., Agents) immediately after first visit
- **Expected**: Page navigates immediately
- **Actual**: Previous page content remains visible for 2-5 seconds while Next.js compiles the new route; "Compiling..." toast appears at bottom
- **Impact**: Dev-mode UX only; would not affect production builds

### Issue #2 (Low/Dev-mode): Sidebar link click appears to not navigate
- **Severity**: Low (dev-mode only)
- **Steps**: Click "Settings" in sidebar from Dashboard
- **Expected**: Settings page loads
- **Actual**: Dashboard stays visible; URL changes but page content doesn't update until compilation completes
- **Note**: Same root cause as Issue #1

### Issue #3 (Medium): Org creation doesn't redirect to setup page
- **Severity**: Medium
- **Steps**: Create organisation via sidebar dropdown > "Create Organisation" dialog
- **Expected**: Redirected to `/orgs/<orgId>/setup` with animated progress steps
- **Actual**: Dialog closes, stays on current page (Settings > General showing old org). Sidebar still shows old org name until page reload.
- **Impact**: Users don't see the setup progress flow; must manually navigate

### Issue #4 (Medium): Harness dropdowns show raw IDs instead of names
- **Severity**: Medium
- **Location**: Settings > General > Organisation section
- **Steps**: View Default Harness and Base Harness dropdowns after page navigation
- **Expected**: Show harness names like "Generic" and "Base"
- **Actual**: Show raw IDs like `harness_01933b5a000070008000000000000602`
- **Note**: This appeared intermittently - on initial load they showed correctly ("Generic"/"Base"), but after org creation they showed IDs

### Issue #5 (Medium): Org dropdown doesn't refresh after creating new org
- **Severity**: Medium
- **Steps**: Create a new org via dropdown > "Create Organisation"
- **Expected**: New org appears in dropdown immediately
- **Actual**: Dropdown still shows only old org(s); new org appears only after full page reload
- **Impact**: Users think org creation failed

### Issue #6 (Medium): Auto-switch to newly created org doesn't work
- **Severity**: Medium
- **Steps**: Create new org
- **Expected**: Automatically switch to new org context
- **Actual**: Stays on previous org; sidebar shows old org name

### Issue #7 (Info): Ctrl+K keyboard shortcut for command palette
- **Severity**: Info (may be headless browser limitation)
- **Steps**: Press Ctrl+K on dashboard
- **Expected**: Command palette opens
- **Actual**: No visible response
- **Note**: Clicking the "Search... Cmd+K" button works correctly. This may be a Chromium headless keyboard event limitation rather than a real bug.

### Issue #8 (Low): MCP Server "Create Server" button enables without URL
- **Severity**: Low
- **Steps**: Open "Add MCP Server" dialog, fill Name only, leave URL empty
- **Expected**: Create button stays disabled (URL is required)
- **Actual**: Create button enables; only browser-level `required` validation prevents actual submission
- **Impact**: Minor UX inconsistency; the form still validates correctly on submit

### Issue #9 (Medium): MCP Server archive has no confirmation dialog
- **Severity**: Medium
- **Steps**: Click "Archive" button on an MCP server card
- **Expected**: Confirmation dialog ("Are you sure?")
- **Actual**: Server immediately archived without confirmation
- **Impact**: Accidental clicks could remove servers; no undo mechanism visible

### Issue #10 (Info): Chat sidebar shows warning badge
- **Severity**: Info
- **Steps**: Navigate to any page; observe Chat item in sidebar
- **Expected**: No warning indicator
- **Actual**: Orange triangle warning badge next to "Chat"
- **Note**: Likely indicates missing LLM API key configuration for the chat model

### Issue #11 (Low): Schedules table requires horizontal scrolling
- **Severity**: Low
- **Location**: Durable > Schedules page
- **Steps**: View schedules list at default viewport (1280x720)
- **Expected**: All columns visible without scrolling
- **Actual**: Target, Last Triggered, Next Trigger, and Actions columns are cut off; horizontal scrollbar appears
- **Impact**: Key information (next trigger time, actions) not visible without scrolling

### Issue #12 (High): Schedule creation from UI silently fails
- **Severity**: High
- **Steps**: Click "New Schedule", fill all fields, click "Create Schedule"
- **Expected**: Schedule created and appears in list
- **Actual**: Dialog closes but schedule is NOT created; no error message shown; list still shows only pre-existing schedules
- **Verification**: Creating same schedule via API works correctly; schedule appears in list after Refresh
- **Impact**: Users cannot create scheduled tasks through the UI
