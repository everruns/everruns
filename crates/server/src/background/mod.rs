//! Server-owned background work: periodic sweeps, retention and GC jobs,
//! cluster-once maintenance, durable-task reaping, and the supervisor that
//! keeps these control-plane loops running.

// Provider-side lifecycle of OpenAI Agents API sessions (EVE-1126)
pub mod actor_history_retention;
pub mod agents_api_lifecycle;

// Object-storage blob garbage collector
pub mod blob_gc;

// Cluster-once maintenance jobs on durable schedules
pub mod cluster_jobs;

// Stale durable-task reaping (non-turn activities)
pub mod durable_reaper;

// Event and sandbox-history retention jobs
pub mod event_retention;

// Pauses managed Sandboxes whose in-process idle timer was lost
pub mod sandbox_idle_sweep;

// Stops, then deletes, managed provider sandboxes no record tracks
pub mod sandbox_orphan_scan;

// Projects sessions whose trace index is behind their events
pub mod session_trace_backfill;

// Resumes sessions left active with no actor leased or runnable
pub mod stranded_actor_sweep;

// Session schedule poller
pub mod session_scheduler;

// Probes a scheduled monitor runs itself, waking the agent only on its condition
pub mod monitor_probe;

// Retained background-task supervision for server-owned control-plane loops
pub mod supervised_task;

// Durable system schedules (leased-resource cleanup, session-task reaper)
pub mod system_schedules;

// Time out sessions stuck in waiting_for_tool_results
pub mod tool_result_timeout;
