// Usage domain — LLM usage and cost accounting.
//
// Records each `llm.generation` into `llm_generations` and the session/agent
// totals (`tracking`), settles usage that arrives after the turn
// (`agents_api`, OpenAI Agents API), and reconciles provider-reported cost
// (`generation_reconciler` over `openrouter_generation`). Budget debits stay
// with `domains::budgets`.

pub mod agents_api;
pub mod generation_reconciler;
pub mod openrouter_generation;
pub mod tracking;

pub use agents_api::AgentsApiUsageReconciler;
pub use generation_reconciler::GenerationReconcilerService;
pub use tracking::UsageTrackingListener;
