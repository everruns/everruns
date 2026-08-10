# TC012: Chats - Thread Switching and Inline Run Cards

## Description

Verify that switching threads does not leak transcript state between them, and that a reply which
started a run renders an inline run card in the turn that started it.

## Preconditions

- Server running (`just start-dev`)
- User logged in
- LLM API keys configured
- Two chat threads with different transcripts, one bound to an agent that can delegate work
  (subagent or background task)

## Test Data

| Thread | User Message |
|--------|-------------|
| A | Remember the number 42. |
| B | Delegate a subagent to summarize today's sessions. |

## Steps

1. Open thread A, send its message, wait for the reply
2. Switch to thread B via the sidebar
3. Observe the transcript immediately after the switch
4. Send thread B's message and wait for the run to start
5. Observe the transcript at the end of that turn
6. Switch back to thread A and then forward to thread B again

## Expected Result

| Check | Expected |
|-------|----------|
| No leak | Thread B never shows thread A's messages, not even briefly during the switch |
| Run card | The turn that started the run renders a card with a status dot, the run name, and a duration |
| Subagent count | The card shows the subagent count when the run has children |
| Live status | The status dot and duration update as the run progresses, then settle on the outcome |
| Open session | The card's **Open session** link opens the run's own session |
| Placement | The card sits at the end of the turn that started it, not at the end of the transcript |
| Re-entry | Switching away and back shows the same card in the same place |
