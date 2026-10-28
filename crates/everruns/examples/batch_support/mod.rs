//! Provider selection shared by the real-API offline examples.
use everruns::Provider;
use std::error::Error;

#[cfg(any(feature = "anthropic", feature = "openai", feature = "gemini"))]
pub fn provider() -> Result<(Provider, String, &'static str), Box<dyn Error>> {
    let default = if cfg!(feature = "anthropic") {
        "anthropic"
    } else if cfg!(feature = "openai") {
        "openai"
    } else {
        "gemini"
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
            _ => return Err(
                "set BATCH_PROVIDER to anthropic, openai or gemini and enable its Cargo feature"
                    .into(),
            ),
        };
    Ok((provider, name, model))
}

#[cfg(not(any(feature = "anthropic", feature = "openai", feature = "gemini")))]
pub fn provider() -> Result<(Provider, String, &'static str), Box<dyn Error>> {
    Err("enable the anthropic, openai or gemini Cargo feature".into())
}
