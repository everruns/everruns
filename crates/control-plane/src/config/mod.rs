// Configuration loading and parsing
//
// This module handles loading built-in and external configuration for LLM providers
// and models. Config providers are read-only and merged with database providers.

pub mod providers;

pub use providers::{load_providers_config, model_with_provider_to_model, ProvidersConfig};
