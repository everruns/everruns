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

- **TC001 Multi-turn Agent Conversation**: PASS - Created "Dad Jokes" agent, opened new session, sent turn 1 message ("Tell me a dad joke about the current time of day"), received contextual dad joke response. Sent turn 2 ("give me 10 more"), received multiple jokes on varied topics. All 4 messages visible in order. Session persisted after page refresh with all messages intact.

### Evidence

| Step | Screenshot |
|------|-----------|
| Agents page with Dad Jokes | `/tmp/test_agent_chat_tc001_step1_agents_page.png` |
| New session (empty) | `/tmp/test_agent_chat_tc001_step3_new_session.png` |
| Turn 1 response complete | `/tmp/test_agent_chat_tc001_step5_turn1_complete.png` |
| Turn 2 response complete | `/tmp/test_agent_chat_tc001_step8_turn2_complete.png` |
| After page refresh (persistence) | `/tmp/test_agent_chat_tc001_step10_persist.png` |
