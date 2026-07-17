//! `everruns-provider` — the provider/LLM abstraction foundation.
//!
//! This crate holds the driver-facing types and traits shared by
//! `everruns-core` and the individual provider crates (OpenAI, Anthropic,
//! Gemini, …): the `ChatDriver` interface, the shared OpenAI/OpenResponses
//! protocol drivers, model profiles, retry/stream helpers, typed IDs, the
//! credential form schema, and the LLM error taxonomy.
//!
//! The goal is that provider crates depend on this crate instead of on
//! `everruns-core`, so a provider is a pure `ChatDriver` implementation with no
//! dependency on core's agent-loop runtime.
//!
//! `everruns-core` depends on this crate and re-exports these modules at their
//! original paths, so existing `everruns_core::…` imports keep working. The
//! adapters that convert core's agent-loop domain types (`Message`,
//! `RuntimeAgent`, `ResolvedModel`) into these driver types live in
//! `everruns-core` (`llm_conversions`), keeping the dependency one-directional.

pub mod credential_schema;
pub mod driver_helpers;
pub mod driver_registry;
pub mod error;
pub mod execution_phase;
pub mod llm_retry;
pub mod model;
pub mod model_profiles;
pub mod openai_protocol;
pub mod openresponses_protocol;
pub mod openresponses_types;
pub mod provider;
pub mod stream_accumulator;
pub mod stream_reconnect;
pub mod tool_types;
pub mod typed_id;
pub mod url_validation;
pub mod user_facing_error;
