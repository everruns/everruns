# Scheduled tasks (UI)

* [TC001: Create Schedule - Creation and Validation](TC001_create_schedule_basic.md) - Verify durable schedule creation, required fields, invalid JSON recovery, and persisted target and enabled state.
* [TC002: Manual Trigger](TC002_manual_trigger.md) - Verify confirmation cancellation has no effect and an accepted manual trigger creates one observable execution of the configured workflow.
* [TC003: Pause and Resume Schedule](TC003_pause_resume.md) - Verify pausing prevents future automatic triggers, resuming restores them, and pause does not cancel executions already admitted.
* [TC004: Execution History and Filtering](TC004_execution_history.md) - Verify execution history preserves identifiers, terminal outcomes, error details, filtering, and workflow navigation.
