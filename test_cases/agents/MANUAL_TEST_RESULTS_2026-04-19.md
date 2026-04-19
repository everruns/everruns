# Manual Agent Test Results - 2026-04-19

## Environment

- **Category:** `test_cases/agents/platform_chat`
- **Stack:** `PORT_PREFIX=272 just start-dev --no-watch`
- **App URL:** `http://localhost:27200`
- **API URL:** `http://localhost:27200/api`
- **MCP URL:** `http://localhost:27200/mcp`
- **Auth Mode:** `none` (DEV_MODE)
- **Model setup during run:**
  - Initial org default model: `GPT-5.4` seed model without API key
  - Retest fallback: org default model changed to enabled `llmsim-default`
- **Browser:** headless Chromium via `agent-browser`

## Test Summary

| Category | Tests | Pass | Fail/Partial | Issues |
|----------|-------|------|-------------|--------|
| platform_chat | 2 | 0 | 2 | 3 |
| **Total** | **2** | **0** | **2** | **3** |

## Detailed Results

### platform_chat (0/2 PASS)

- **TC002 Discover and Execute via MCP Tier 2 Tools**: PARTIAL
  - `discover` with `agent` returned catalog entries including `create_agent`.
  - `discover` with `harnesses` returned `list_harnesses`.
  - `discover` without a query returned the expected validation error.
  - `execute` with `list_harnesses` returned seeded harnesses including `generic`.
  - `execute` with an invalid builtin returned `bash: definitely_not_a_real_command: command not found`.
  - `execute` with `create_agent --name 'catalog-smoke-bot' --display_name 'Catalog Smoke Bot' --system_prompt 'Reply with the single word catalog.'` returned `create_agent: callback failed`, but `list_agents` immediately after showed that the agent had in fact been created. This leaves the case in a partial-fail state because mutation and reported result disagree.

- **TC003 Answer Platform Questions from Embedded Docs**: FAIL
  - Browser run could not reach usable `/chat` content in this auth-none dev stack. `http://localhost:27200/chat` redirected to `/login`, and the rendered login page was effectively blank. Screenshot captured at `test_cases/agents/platform_chat/evidence/TC003_blank_login_2026-04-19.png`.
  - API fallback confirmed Platform Chat session creation works (`POST /api/v1/sessions/chat` returned the `platform-chat` harness session).
  - First turn failed before tool use because the org default OpenAI model had no API key (`LLM error: API key is required. Configure the API key in provider settings.` from session events).
  - After enabling `llmsim-default` and setting it as the org default model, Platform Chat responded with the canned text `Hello! I'm a simulated LLM response.` and made no docs/file tool calls. This does not validate embedded-docs retrieval and therefore fails the case.

## Issues Found

### Issue 1 (Medium): `execute create_agent` reports failure after successful mutation
- **Area:** MCP Tier 2 `execute`
- **Steps:** Call `execute` with `create_agent --name 'catalog-smoke-bot' --display_name 'Catalog Smoke Bot' --system_prompt 'Reply with the single word catalog.'`
- **Expected:** Tool returns created agent JSON
- **Actual:** Tool returns `create_agent: callback failed`, but `list_agents` shows the agent was created
- **Impact:** External MCP clients cannot trust `execute` success/failure semantics for mutations

### Issue 2 (Medium): `/chat` redirects to unusable `/login` page in auth-none dev mode
- **Area:** UI / Platform Chat entry
- **Steps:** Open `http://localhost:27200/chat` in DEV_MODE (`auth_mode: none`)
- **Expected:** Platform Chat should open directly or show a usable auth-none path
- **Actual:** Redirect lands on `/login`; rendered page is effectively blank except for the dev-tools button
- **Impact:** Manual browser validation of Platform Chat is blocked in local auth-none stacks

### Issue 3 (Info): Embedded-docs case requires a real tool-calling model, not `llmsim-default`
- **Area:** Platform Chat test environment
- **Steps:** Run TC003 with `llmsim-default`
- **Expected:** Agent consults `/workspace/docs` via file or bash tools
- **Actual:** `llmsim-default` returns a fixed canned response with no tool calls
- **Impact:** TC003 cannot be meaningfully validated in a no-key dev environment unless a real provider is configured

## Evidence

| Artifact | Location |
|----------|----------|
| Blank login screenshot | `/Users/mykhailochalyi/Projects/everruns/everruns-platform-chat-tests/test_cases/agents/platform_chat/evidence/TC003_blank_login_2026-04-19.png` |
| New test cases | `/Users/mykhailochalyi/Projects/everruns/everruns-platform-chat-tests/test_cases/agents/platform_chat/TC002_discover_and_execute_via_mcp.md` |
| New test cases | `/Users/mykhailochalyi/Projects/everruns/everruns-platform-chat-tests/test_cases/agents/platform_chat/TC003_answer_from_embedded_platform_docs.md` |

## Notes

- TC003 was executed through the API after the UI path failed, because the goal was to validate the `platform-chat` harness behavior rather than stop at the login-page issue.
- The org default model was temporarily switched to `llmsim-default` during execution so Platform Chat could produce any response at all in this local environment.
