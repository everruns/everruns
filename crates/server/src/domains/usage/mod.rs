// Usage domain — LLM usage and cost accounting.
//
// Records each `llm.generation` into `llm_generations` and the session/agent
// totals (`tracking`), and reconciles provider-reported cost
// (`generation_reconciler` over `openrouter_generation`). Budget debits stay
// with `domains::budgets`.

pub mod generation_reconciler;
pub mod openrouter_generation;
pub mod tracking;

pub use generation_reconciler::GenerationReconcilerService;
pub use tracking::UsageTrackingListener;
