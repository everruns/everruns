#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Everruns model vendor drivers, one module and one feature per vendor.
//!
//! `everruns-drivers` is part of the [Everruns](https://everruns.com)
//! ecosystem. Each module implements the [`ChatDriver`] contract from
//! `everruns-provider` for one vendor and registers it into a
//! [`DriverRegistry`]. Vendors whose API is OpenAI-compatible wrap one of that
//! crate's shared protocol drivers and add only identity, credentials, auth,
//! base URL, and model discovery; vendors with their own wire (Anthropic,
//! Gemini, Bedrock) carry their request and response types in their module.
//!
//! # Features
//!
//! Every vendor is behind a feature and none is on by default, so a consumer
//! compiles and ships only the vendors it serves:
//!
//! ```toml
//! everruns-drivers = { version = "0.35", features = ["openai", "anthropic"] }
//! ```
//!
//! | Feature | Module | Driver | Wire protocol |
//! | --- | --- | --- | --- |
//! | `anthropic` | [`anthropic`] | Anthropic Claude | Anthropic Messages |
//! | `bedrock` | [`bedrock`] | AWS Bedrock | Bedrock Converse |
//! | `bedrock-default-credentials` | [`bedrock`] | AWS default credential chain for Bedrock | |
//! | `cloudflare` | [`cloudflare`] | Cloudflare AI Gateway | OpenAI Chat Completions |
//! | `fireworks` | [`fireworks`] | Fireworks AI | OpenAI Chat Completions |
//! | `gemini` | [`gemini`] | Google Gemini | Gemini API |
//! | `mai` | [`mai`] | Microsoft AI (Foundry) | OpenAI Chat Completions |
//! | `meta` | [`meta`] | Meta Model API | Open Responses |
//! | `openai` | [`openai`] | OpenAI and Azure OpenAI | Responses and Chat Completions |
//! | `openrouter` | [`openrouter`] | OpenRouter | OpenAI Responses-compatible |
//! | `vercel` | [`vercel`] | Vercel AI Gateway | Open Responses |
//!
//! The `everruns` facade re-exports this crate as `everruns::drivers`, and its
//! vendor features turn on the matching features here.
//!
//! # Moved crates
//!
//! `everruns-anthropic`, `everruns-bedrock`, `everruns-fireworks`,
//! `everruns-gemini`, `everruns-mai`, `everruns-meta`, `everruns-openai` and
//! `everruns-openrouter` are now the modules of the same name here. Replace
//! the dependency with this crate and the vendor's feature, and
//! `everruns_openai::X` with `everruns_drivers::openai::X`.

#[cfg(feature = "anthropic")]
pub mod anthropic;
#[cfg(feature = "bedrock")]
pub mod bedrock;
#[cfg(feature = "cloudflare")]
pub mod cloudflare;
#[cfg(feature = "fireworks")]
pub mod fireworks;
#[cfg(feature = "gemini")]
pub mod gemini;
#[cfg(feature = "mai")]
pub mod mai;
#[cfg(feature = "meta")]
pub mod meta;
#[cfg(feature = "openai")]
pub mod openai;
#[cfg(feature = "openrouter")]
pub mod openrouter;
#[cfg(feature = "vercel")]
pub mod vercel;

// Re-export core types for convenience.
pub use everruns_contracts::driver_registry::{ChatDriver, DriverRegistry};

/// Register every driver this crate's enabled features provide.
///
/// A host that enables a feature gets that vendor without editing its
/// registration code, which is the point of keeping them in one crate.
///
/// # Example
///
/// ```
/// use everruns_drivers::{register_drivers, DriverRegistry};
///
/// let mut registry = DriverRegistry::new();
/// register_drivers(&mut registry);
/// ```
#[cfg_attr(
    not(any(
        feature = "anthropic",
        feature = "bedrock",
        feature = "cloudflare",
        feature = "fireworks",
        feature = "gemini",
        feature = "mai",
        feature = "meta",
        feature = "openai",
        feature = "openrouter",
        feature = "vercel"
    )),
    allow(unused_variables)
)]
pub fn register_drivers(registry: &mut DriverRegistry) {
    #[cfg(feature = "anthropic")]
    anthropic::register_driver(registry);
    #[cfg(feature = "bedrock")]
    bedrock::register_driver(registry);
    #[cfg(feature = "cloudflare")]
    cloudflare::register_driver(registry);
    #[cfg(feature = "fireworks")]
    fireworks::register_driver(registry);
    #[cfg(feature = "gemini")]
    gemini::register_driver(registry);
    #[cfg(feature = "mai")]
    mai::register_driver(registry);
    #[cfg(feature = "meta")]
    meta::register_driver(registry);
    #[cfg(feature = "openai")]
    openai::register_driver(registry);
    #[cfg(feature = "openrouter")]
    openrouter::register_driver(registry);
    #[cfg(feature = "vercel")]
    vercel::register_driver(registry);
}
