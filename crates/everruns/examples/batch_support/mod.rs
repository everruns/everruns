//! Provider selection shared by the real-API offline examples.
use everruns::Provider;
use std::error::Error;

#[cfg(any(
    feature = "anthropic",
    feature = "openai",
    feature = "gemini",
    feature = "mistral",
    feature = "groq",
    feature = "fireworks"
))]
pub fn provider() -> Result<(Provider, String, &'static str), Box<dyn Error>> {
    let default = if cfg!(feature = "anthropic") {
        "anthropic"
    } else if cfg!(feature = "openai") {
        "openai"
    } else if cfg!(feature = "gemini") {
        "gemini"
    } else if cfg!(feature = "mistral") {
        "mistral"
    } else if cfg!(feature = "groq") {
        "groq"
    } else {
        "fireworks"
    };
    let name = std::env::var("BATCH_PROVIDER").unwrap_or_else(|_| default.into());
    let (provider, model) =
        match name.as_str() {
            #[cfg(feature = "anthropic")]
            "anthropic" => (
                everruns::providers::anthropic::Anthropic::from_env()?.into(),
                "claude-sonnet-5",
            ),
            #[cfg(feature = "openai")]
            "openai" => (
                everruns::providers::openai::OpenAI::from_env()?.into(),
                "gpt-6.1-sol",
            ),
            #[cfg(feature = "gemini")]
            "gemini" => (
                everruns::providers::gemini::Gemini::from_env()?.into(),
                "gemini-4-argon",
            ),
            #[cfg(feature = "mistral")]
            "mistral" => (everruns::providers::mistral::Mistral::from_env()?.into(), "mistral-small-latest"),
            #[cfg(feature = "groq")]
            "groq" => (everruns::providers::groq::Groq::from_env()?.into(), "llama-3.1-8b-instant"),
            #[cfg(feature = "fireworks")]
            "fireworks" => {
                use everruns::providers::fireworks::{Fireworks, FireworksBatchConfig};
                let config = FireworksBatchConfig::new(std::env::var("FIREWORKS_ACCOUNT_ID")?)?
                    .artifact_origin(&std::env::var("FIREWORKS_ARTIFACT_ORIGIN")?)?;
                (Fireworks::from_env()?.batch(config).into(), "accounts/fireworks/models/llama-v3p1-8b-instruct")
            },
            _ => return Err(
                "set BATCH_PROVIDER to anthropic, openai, gemini, mistral, groq or fireworks and enable its Cargo feature"
                    .into(),
            ),
        };
    Ok((provider, name, model))
}

#[cfg(not(any(
    feature = "anthropic",
    feature = "openai",
    feature = "gemini",
    feature = "mistral",
    feature = "groq",
    feature = "fireworks"
)))]
pub fn provider() -> Result<(Provider, String, &'static str), Box<dyn Error>> {
    Err("enable the anthropic, openai, gemini, mistral, groq or fireworks Cargo feature".into())
}
