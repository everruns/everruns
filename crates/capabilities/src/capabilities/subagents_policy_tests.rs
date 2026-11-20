//! Pure status-mapping and nesting-policy tests for the subagents capability.

use super::*;

#[test]
fn terminal_subagent_status_maps_only_terminal_wait_states() {
    // `idle` is not terminal: a failed turn also idles the worker, so it
    // must not settle the subagent as completed.
    assert_eq!(terminal_subagent_status("idle"), None);
    assert_eq!(
        terminal_subagent_status("completed"),
        Some(everruns_core::session::SubagentStatus::Completed)
    );
    assert_eq!(
        terminal_subagent_status("failed"),
        Some(everruns_core::session::SubagentStatus::Failed)
    );
    assert_eq!(
        terminal_subagent_status("cancelled"),
        Some(everruns_core::session::SubagentStatus::Cancelled)
    );
    assert_eq!(
        terminal_subagent_status("sealed"),
        Some(everruns_core::session::SubagentStatus::Sealed)
    );
    assert_eq!(
        terminal_subagent_task_state(&everruns_core::session::SubagentStatus::Completed),
        SessionTaskState::Succeeded
    );
    // A sealed subagent settles as a terminal, non-retryable failed task.
    assert_eq!(
        terminal_subagent_task_state(&everruns_core::session::SubagentStatus::Sealed),
        SessionTaskState::Failed
    );
    assert_eq!(
        terminal_subagent_task_state(&everruns_core::session::SubagentStatus::Cancelled),
        SessionTaskState::Canceled
    );
    assert_eq!(
        terminal_subagent_task_state(&everruns_core::session::SubagentStatus::MaxIterationsReached),
        SessionTaskState::Failed
    );
    assert_eq!(terminal_subagent_status("waiting_for_tool_results"), None);
    assert_eq!(terminal_subagent_status("paused"), None);
}

#[test]
fn subagent_nesting_policy_resolves_platform_org_agent_precedence() {
    let platform = everruns_core::delegation_services::SubagentNestingPolicy::default()
        .with_platform_default(4);
    assert_eq!(platform.max_subagent_depth(), 4);

    let org = platform.with_org_override(Some(3));
    assert_eq!(org.max_subagent_depth(), 3);

    let agent = org.with_agent_override(Some(1));
    assert_eq!(agent.max_subagent_depth(), 1);
}
