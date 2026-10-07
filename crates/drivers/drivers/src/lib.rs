#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
#![doc = include_str!("../README.md")]

#[cfg(feature = "anthropic")]
pub mod anthropic;
#[cfg(feature = "bedrock")]
pub mod bedrock;
#[cfg(feature = "chatgpt")]
pub mod chatgpt;
#[cfg(feature = "cloudflare")]
pub mod cloudflare;
#[cfg(feature = "codex")]
pub mod codex;
#[cfg(feature = "fireworks")]
pub mod fireworks;
#[cfg(feature = "gemini")]
pub mod gemini;
#[cfg(feature = "mai")]
pub mod mai;
#[cfg(feature = "meta")]
pub mod meta;
#[cfg(feature = "mistral")]
pub mod mistral;
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
        feature = "chatgpt",
        feature = "codex",
        feature = "cloudflare",
        feature = "fireworks",
        feature = "gemini",
        feature = "mai",
        feature = "meta",
        feature = "mistral",
        feature = "openai",
        feature = "openrouter",
        feature = "vercel"
    )),
    allow(unused_variables)
)]
pub fn register_drivers(registry: &mut DriverRegistry) {
    #[cfg(feature = "chatgpt")]
    chatgpt::register_driver(registry);
    #[cfg(feature = "codex")]
    codex::register_driver(registry);
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
    #[cfg(feature = "mistral")]
    mistral::register_driver(registry);
    #[cfg(feature = "openai")]
    openai::register_driver(registry);
    #[cfg(feature = "openrouter")]
    openrouter::register_driver(registry);
    #[cfg(feature = "typesafe")]
    typesafe::register_driver(registry);
    #[cfg(feature = "vercel")]
    vercel::register_driver(registry);
}

#[cfg(any(feature = "typesafe", feature = "openrouter"))]
pub mod systemone;
#[cfg(feature = "typesafe")]
pub mod typesafe;
