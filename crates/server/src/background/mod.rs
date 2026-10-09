//! Server-owned background work: periodic sweeps, retention and GC jobs,
//! cluster-once maintenance, durable-task reaping, and the supervisor that
//! keeps these control-plane loops running.

// Provider-side lifecycle of OpenAI Agents API sessions (EVE-1126)
pub mod agents_api_lifecycle;

// Object-storage blob garbage collector
pub mod blob_gc;

// Cluster-once maintenance jobs on durable schedules
pub mod cluster_jobs;

// Surface sealed and exhausted durable turns (forward-progress guard, EVE-534)
// to sessions; driven by the stale-task reaper.
pub mod durable_failure;
pub mod durable_reaper;
pub mod durable_seal;

// Event and sandbox-history retention jobs
pub mod event_retention;
pub mod sandbox_history_retention;

// Session schedule poller
pub mod session_scheduler;

// Retained background-task supervision for server-owned control-plane loops
pub mod supervised_task;

// Durable system schedules (leased-resource cleanup, session-task reaper)
pub mod system_schedules;

// Time out sessions stuck in waiting_for_tool_results
pub mod tool_result_timeout;
