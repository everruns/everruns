---
name: manual-ui-testing
description: Run manual UI test cases using agent-browser against a running stack. Use when the user asks to run UI tests, test the UI, run manual tests, or verify UI behavior.
user-invocable: true
allowed-tools: Bash(npx agent-browser:*), Bash(agent-browser:*), Bash(just:*), Bash(doppler:*)
---

# Manual UI Testing

Goal: execute UI test cases from `test_cases/ui/` using `agent-browser`, record results, and file issues for failures.

## When To Use

- User asks to run UI tests, manual tests, or test the UI
- User asks to verify a specific UI flow or feature
- User asks to re-test after a fix

## Prerequisites

### 1. Running Stack

Full auth mode requires the full stack (not DEV_MODE). Start with a unique `PORT_PREFIX`:

```bash
PORT_PREFIX=<prefix> doppler run -- just start-all
```

Wait for all services (PostgreSQL, Valkey, API, Worker, UI, Caddy) to be healthy. Verify:

```bash
curl -s http://localhost:<prefix>00/healthz
```

If the stack is already running, confirm the PORT_PREFIX and auth mode before proceeding.

### 2. Agent-Browser

`agent-browser` runs headless Chromium. No special install needed — it's available via npx. The browser daemon persists between commands within a session.

## Execution Flow

### Step 1: Identify Test Scope

Ask the user or determine from context which test categories to run:

| Category | Path | Requires |
|----------|------|----------|
| admin_login | `test_cases/ui/admin_login/` | AUTH_MODE=admin |
| full_auth | `test_cases/ui/full_auth/` | AUTH_MODE=full |
| org_creation | `test_cases/ui/org_creation/` | Authenticated user |
| mcp_servers | `test_cases/ui/mcp_servers/` | Authenticated + org |
| global_chat | `test_cases/ui/global_chat/` | Authenticated + org |
| global_search | `test_cases/ui/global_search/` | Authenticated + org |
| scheduled_tasks | `test_cases/ui/scheduled_tasks/` | Authenticated + org + agent |

If no specific scope requested, run all categories. Prioritize by dependency order: auth → org → features.

### Step 2: Read Test Cases

Read each `.md` file in the target category. Each test case has:
- **Preconditions**: Required state (auth mode, existing data)
- **Test Data**: Specific values to use
- **Steps**: Numbered actions
- **Expected Result**: Pass criteria

### Step 3: Execute with agent-browser

Core pattern for each test:

```bash
# Navigate
agent-browser open http://localhost:<prefix>00/<path>
agent-browser wait --load networkidle

# Discover elements
agent-browser snapshot -i
# Output: @e1 [input type="email"], @e2 [button] "Submit", etc.

# Interact using refs
agent-browser fill @e1 "value"
agent-browser click @e2
agent-browser wait --load networkidle

# Verify result
agent-browser snapshot -i
agent-browser screenshot /tmp/test_<category>_<tc>.png
```

**Key patterns:**

- **Login flow**: Navigate to login page → snapshot → fill email/password → click submit → wait → snapshot to verify redirect
- **Form submission**: Navigate → snapshot → fill fields → click submit → wait → snapshot to verify success/error
- **Navigation**: Click sidebar/menu refs → wait for networkidle → snapshot to verify page content
- **Validation**: Fill partial data → attempt submit → verify error messages appear

**Hints from experience:**

- Always `wait --load networkidle` after navigation and form submissions
- Re-snapshot after every navigation or DOM change — refs (`@e1`, etc.) are invalidated
- Chain independent commands with `&&` for efficiency: `agent-browser fill @e1 "x" && agent-browser fill @e2 "y"`
- Don't chain when you need to read snapshot output to determine next refs
- In dev mode, Next.js compilation can delay page loads 2-5s — add extra waits if needed
- Ctrl+K and other keyboard shortcuts may not work in headless Chromium — test via click instead
- Take a screenshot at each significant step for evidence, not just pass/fail

### Step 4: Record Results

Create or update `test_cases/ui/MANUAL_TEST_RESULTS_<date>.md` with:

```markdown
# Manual UI Test Results - <YYYY-MM-DD>

## Environment

- **Auth Mode**: <admin|full>
- **Stack**: <components running>
- **PORT_PREFIX**: <value>
- **Browser**: Chromium (headless, via agent-browser)

## Test Summary

| Category | Tests | Pass | Fail/Partial | Issues |
|----------|-------|------|-------------|--------|
| ... | ... | ... | ... | ... |
| **Total** | **N** | **N** | **N** | **N** |

## Detailed Results

### <Category> (N/M PASS)

- **TC001 <Name>**: PASS|FAIL|PARTIAL - <one-line description of what happened>

## Issues Found

### Issue #N (<Severity>): <Title>
- **Severity**: Low|Medium|High|Info
- **Steps**: How to reproduce
- **Expected**: What should happen
- **Actual**: What happened
- **Impact**: User-facing consequence
```

### Step 5: File Issues (Optional)

If the user asks to file issues for failures, use Linear MCP tools:
- Team: EVE, Project: OSS
- Include severity, repro steps, expected vs actual
- Reference the test case ID (e.g., "org_creation/TC003")

## Partial Runs

If the user asks to test a single feature or re-test a specific case:
1. Read just that test case file
2. Set up preconditions (may need to run auth tests first)
3. Execute and record
4. Append to or update the existing results file

## Troubleshooting

| Problem | Solution |
|---------|----------|
| `agent-browser` not found | Run via `npx agent-browser` |
| Stale refs after click | Always re-snapshot after DOM changes |
| Page doesn't load | Check stack health: `curl localhost:<prefix>00/healthz` |
| Login redirect loop | Verify AUTH_MODE env var matches test category |
| Screenshots blank | Add `wait --load networkidle` before screenshot |
| Element not visible | Try `agent-browser scroll down` before snapshot |
