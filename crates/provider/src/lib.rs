//! `everruns-provider` — the provider/LLM abstraction foundation.
//!
//! This crate holds the driver-facing types shared by `everruns-core` and the
//! individual provider crates (OpenAI, Anthropic, Gemini, …). It is being grown
//! incrementally: the eventual goal is that provider crates depend on this crate
//! instead of on `everruns-core`, so that a provider is a pure `ChatDriver`
//! implementation with no dependency on core's agent-loop runtime.
//!
//! During the migration, `everruns-core` depends on this crate and re-exports
//! these modules at their original paths, so existing `everruns_core::…` imports
//! keep working unchanged.

pub mod credential_schema;
pub mod openresponses_types;
pub mod typed_id;
pub mod url_validation;
pub mod user_facing_error;
