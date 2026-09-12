# Subagents (API)

* [TC001: Spawn Subagent - Basic](TC001_spawn_agent_subagent_basic.md) - Verify that an agent with the `subagents` capability can spawn a subagent via the `spawn_agent` tool, creating a child session with `parent_session_id` set and returning `subagent_id`, `name`, and `status` in the tool...
* [TC002: List Subagent Tasks After Spawning](TC002_get_subagents_list.md) - Verify that after spawning multiple subagents, `list_tasks` (from the generic session_tasks capability) returns all subagent tasks with correct names, statuses, and count.
* [TC003: Message Task - Send Follow-Up to Subagent](TC003_message_subagent.md) - Verify that `message_task` can send a follow-up message to a subagent's task channel and that the subagent processes the message.
* [TC004: Subagent Nesting Prevention](TC004_subagent_nesting_prevention.md) - Verify that a subagent (a session with `parent_session_id` set) cannot spawn another subagent.
* [TC005: Cancel Task - Subagent Cancellation](TC005_subagent_cancel.md) - Verify that `cancel_task` delivers a cooperative cancellation request to a subagent task and that the task transitions to a canceled state.
