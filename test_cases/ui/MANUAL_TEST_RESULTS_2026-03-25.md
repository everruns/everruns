# Manual UI Test Results - 2026-03-25

## Environment

- **Auth Mode**: None (DEV_MODE)
- **Stack**: Server + UI + Caddy (in-memory, Doppler for LLM keys)
- **PORT_PREFIX**: 271
- **Browser**: Chromium (headless, via agent-browser)

## Test Summary

| Category | Tests | Pass | Fail/Partial | Issues |
|----------|-------|------|-------------|--------|
| agent_chat | 1 | 1 | 0 | 0 |
| **Total** | **1** | **1** | **0** | **0** |

## Detailed Results

### agent_chat (1/1 PASS)

- **TC001 Multi-turn Agent Conversation**: PASS - Used existing "Dad Jokes" agent, opened new session, sent turn 1 message ("Tell me a dad joke about the current time of day"), received contextual dad joke response. Sent turn 2 ("give me 10 more"), received multiple jokes on varied topics. All 4 messages visible in order. Session persisted after page refresh with all messages intact.

### Evidence

| Step | Screenshot |
|------|-----------|
| Agents page with Dad Jokes | Screenshot captured during test execution |
| New session (empty) | Screenshot captured during test execution |
| Turn 1 response complete | Screenshot captured during test execution |
| Turn 2 response complete | Screenshot captured during test execution |
| After page refresh (persistence) | Screenshot captured during test execution |
