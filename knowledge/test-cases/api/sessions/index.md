# Sessions (API)

* [TC001: Create Session for Agent](TC001_create_session_for_agent.md) - Verify that a session can be created for an existing agent and starts in the correct initial state.
* [TC002: Send Message and Get Response](TC002_send_message_and_get_response.md) - Verify that sending a user message to a session triggers a turn, the agent responds, and the session transitions through correct states.
* [TC003: Multi-Turn Conversation](TC003_multi_turn_conversation.md) - Verify that an agent maintains context across multiple turns in the same session.
* [TC004: Session with Tool Use](TC004_session_with_tool_use.md) - Verify that an agent with capabilities uses tools during a session and produces tool call events.
* [TC005: Session List and Filter by Agent](TC005_session_list_and_filter.md) - Verify that sessions can be listed and filtered by agent_id.
* [TC006: Cancel Active Turn](TC006_cancel_active_turn.md) - Verify that an active turn can be cancelled and the session returns to idle state.
* [TC007: Delete Session](TC007_delete_session.md) - Verify that a session can be deleted and is no longer accessible.
* [TC008: Session with Model Override](TC008_session_with_model_override.md) - Verify that a session can override the agent's default model and that the override is used for responses.
