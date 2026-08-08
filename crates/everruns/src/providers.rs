//! Real LLM provider configuration for the facade.
//!
//! Each provider lives behind its own cargo feature so the default facade build
//! stays fully offline — no provider crate, no Reqwest edge. Enable the `openai`
//! feature to configure OpenAI-backed models through
//! [`openai::OpenAI`](crate::providers::openai::OpenAI).

#[cfg(feature = "openai")]
pub mod openai;
