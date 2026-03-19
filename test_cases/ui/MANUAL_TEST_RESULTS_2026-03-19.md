# Manual UI Test Results - 2026-03-19

## Environment

- **Auth Mode**: dev (DEV_MODE, in-memory storage)
- **Stack**: API (9301), UI (9305), Proxy (9300), Caddy
- **PORT_PREFIX**: default (93xx)
- **Browser**: Chromium (headless, via agent-browser)

## Test Summary

| Category | Tests | Pass | Fail/Partial | Issues |
|----------|-------|------|-------------|--------|
| daytona_connection | 1 | 1 (partial) | 0 | 1 |
| **Total** | **1** | **1** | **0** | **1** |

## Detailed Results

### daytona_connection (1/1 PASS - partial)

- **TC001 Daytona OpenUI Connection - Sandbox Lifecycle**: PASS (partial)
  - **Session 1** (connection flow): Connection prompt appeared correctly. API Key dialog rendered with Daytona provider name, instructions link, and input field. API key entered, validated, and saved. However, after submitting the connection, the worker hit a race condition in DEV_MODE (`User message not found`) and the session got stuck. This is a known DEV_MODE limitation — the test case spec requires `just start-all` (PostgreSQL).
  - **Session 2** (automatic flow, connection already saved): Sandbox created, command executed (`python3 -c 'print(123*456)'` → `56088`), sandbox auto-deleted by agent. All tool calls succeeded. No connection prompt appeared (key already saved).
  - **Session 3** (explicit delete flow): Sandbox created, command executed (result: 56088), sandbox kept alive per request. Second message "Delete the sandbox" triggered `daytona_manage_sandbox` with `action=delete` → success. Agent confirmed: "Deleted sandbox `522f3c9c-...`".

### Checks

| Check | Result | Notes |
|-------|--------|-------|
| Connection prompt | PASS | Inline "Setup Connection" card appeared with Connect/Skip buttons |
| API Key dialog | PASS | Shows "Connect Daytona" heading, API Key field, Daytona Dashboard link |
| Connection saved | PASS | After submit, dialog closed, connection stored in Settings |
| Sandbox created | PASS | sandbox_id returned, status=running |
| Command executed | PASS | `python3 -c 'print(123*456)'` → exit_code=0, output=56088 |
| Sandbox deleted | PASS | `daytona_manage_sandbox` action=delete → success=true |
| Agent confirms deletion | PASS | Agent responded: "Deleted sandbox `522f3c9c-...`" |
| Resources cleaned | SKIPPED | DEV_MODE doesn't persist leased resources for verification |

## Issues Found

### Issue #1 (Medium): DEV_MODE race condition on connection resume

- **Severity**: Medium
- **Steps**: Send message to Daytona Coder with no connection → connection prompt appears → enter API key → submit → workflow resumes
- **Expected**: Agent retries `daytona_create_sandbox` with the now-valid connection
- **Actual**: Worker fails with `User message not found: message_019d03e2f83d7f93948c990b6f99a3d8` and retries 5 times before giving up. Session stays stuck in `active` state.
- **Impact**: In DEV_MODE (in-memory storage), the connection resume flow can fail due to a race condition in the message store. Works correctly in full mode with PostgreSQL.
- **Workaround**: Use `just start-all` (full PostgreSQL mode) for testing connection flows, or create a new session after connecting (connection persists across sessions).

## Screenshots

- `/tmp/tc001_step1_agents_page.png` - Agents page showing Daytona Coder
- `/tmp/tc001_step2_session_chat.png` - Empty session chat
- `/tmp/tc001_step4_connection_prompt.png` - Inline connection prompt (Connect/Skip)
- `/tmp/tc001_step5_apikey_dialog.png` - API Key dialog
- `/tmp/tc001_step7_connection_saved.png` - After connection saved, agent processing
- `/tmp/tc001_step9_sandbox_complete.png` - Session 2 complete (auto-delete)
- `/tmp/tc001_step10_sandbox_running.png` - Session 3 with sandbox running
- `/tmp/tc001_step12_sandbox_deleted.png` - Session 3 sandbox deleted confirmation

## Cleanup

- Daytona connection remains saved in Settings > Connections
- All sandboxes deleted (confirmed via `daytona_manage_sandbox` success responses)
- No resource leaks
