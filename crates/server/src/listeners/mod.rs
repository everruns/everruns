// Event listeners with no single owning domain.
//
// Each reacts to the server's event stream off the event path and is
// best-effort: a failure costs a missing summary, metric, or status
// transition, never a run. `app_builder::listeners` wires them into the
// `EventService`. Listeners owned by one domain live with it instead
// (`domains::usage::tracking`, `domains::audit_logs::approval_listener`,
// `domains::mcp_servers::events`).

pub mod coordination;
pub mod run_summary;
pub mod turn_latency;

pub use run_summary::RunSummaryService;
pub use turn_latency::TurnLatencyListener;
