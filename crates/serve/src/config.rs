//! `serve.toml`: the few facts about an app that are not code.
//!
//! ```toml
//! name = "revenue-analyst"          # defaults to the Cargo package name
//!
//! [sandbox]
//! kind = "bashkit"                  # none | local | bashkit | microvm
//!
//! [secrets]
//! required = ["WAREHOUSE_URL"]      # beyond those connections/channels declare
//!
//! [deploy]
//! target = "everruns-cloud"
//!
//! [auth]                            # open when no method is set
//! api_keys = []                     # prefer SERVE_API_KEYS
//!
//! [images]                          # image generation on every agent
//! model = "gpt-image-2.6-flare"     # optional; this is the default
//! format = "png"                    # optional default: png | jpeg | webp | gif | svg
//!
//! [voice]                           # `voice` feature: how calls sound
//! voice = "marin"
//! greeting = "Hi, you are talking to an AI assistant."
//! ```

use serde::{Deserialize, Serialize};

/// Parsed `serve.toml`.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    /// App name; defaults to the binary's Cargo package name.
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub sandbox: SandboxConfig,
    #[serde(default)]
    pub secrets: SecretsConfig,
    #[serde(default)]
    pub deploy: DeployConfig,
    /// `[auth]`: who may call the agent routes. Open when empty.
    #[serde(default)]
    pub auth: crate::auth::AuthConfig,
    /// `[images]`: give every agent `generate_image` and `edit_image`.
    /// Provider keys come from the environment (`OPENAI_API_KEY`,
    /// `OPENROUTER_API_KEY`, ...). Absent means no image tools.
    #[serde(default)]
    pub images: Option<ImagesConfig>,
    /// `[voice]`: settings for every agent's voice channel (`voice` feature).
    /// The platform's voice channel config, so a channel moves between serve
    /// and Everruns unchanged.
    #[cfg(feature = "voice")]
    #[serde(default)]
    pub voice: Option<everruns::voice::VoiceChannelConfig>,
}

/// `[sandbox]`.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SandboxConfig {
    #[serde(default)]
    pub kind: SandboxKind,
}

/// Where the agent's shell and files run. The code never changes between
/// kinds; the host (or `dev`) supplies the adapter.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SandboxKind {
    /// No shell or filesystem tools.
    #[default]
    None,
    /// Files in a real directory under the app's data dir.
    Local,
    /// A virtual bash over the session filesystem (bashkit).
    Bashkit,
    /// An isolated microVM supplied by the host. `dev` substitutes bashkit.
    Microvm,
}

impl SandboxKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            SandboxKind::None => "none",
            SandboxKind::Local => "local",
            SandboxKind::Bashkit => "bashkit",
            SandboxKind::Microvm => "microvm",
        }
    }
}

/// `[images]`: the `image_generation` capability's settings.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ImagesConfig {
    /// Image model; GPT Image 2.6 Flare when unset.
    #[serde(default)]
    pub model: Option<String>,
    /// Default output format when a call picks none.
    #[serde(default)]
    pub format: Option<String>,
}

impl ImagesConfig {
    /// The capability reference agents get. Images are saved under
    /// `/workspace/images`, outside the hidden folders the workspace policy
    /// keeps read-only, so the session's files expose them.
    pub fn capability(&self) -> everruns::CapabilityRef {
        let mut config = serde_json::json!({ "output_dir": "/workspace/images" });
        if let Some(model) = &self.model {
            config["model"] = model.clone().into();
        }
        if let Some(format) = &self.format {
            config["default_format"] = format.clone().into();
        }
        everruns::CapabilityRef::with_config("image_generation", config)
    }
}

/// `[secrets]`.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SecretsConfig {
    #[serde(default)]
    pub required: Vec<String>,
}

/// `[deploy]`.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DeployConfig {
    #[serde(default)]
    pub target: Option<String>,
}

impl AppConfig {
    pub fn parse(text: &str) -> crate::Result<Self> {
        Ok(toml::from_str(text)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "voice")]
    #[test]
    fn parses_voice_settings_over_defaults() {
        let config = AppConfig::parse(
            r#"
            [voice]
            voice = "cedar"
            greeting = "Hello"
            "#,
        )
        .unwrap();
        let voice = config.voice.unwrap();
        assert_eq!(voice.voice, "cedar");
        assert_eq!(voice.greeting.as_deref(), Some("Hello"));
        assert_eq!(voice.model, "gpt-realtime-2");
    }

    #[test]
    fn parses_images_into_a_capability() {
        let config = AppConfig::parse(
            r#"
            [images]
            model = "gpt-image-2.6-sunburst"
            format = "gif"
            "#,
        )
        .unwrap();
        let capability = config.images.unwrap().capability();
        assert_eq!(capability.capability_id(), "image_generation");
        assert_eq!(
            capability.config_value(),
            &serde_json::json!({
                "model": "gpt-image-2.6-sunburst",
                "default_format": "gif",
                "output_dir": "/workspace/images"
            })
        );
        assert!(AppConfig::parse("").unwrap().images.is_none());
    }

    #[test]
    fn parses_auth_methods_in_the_channel_shape() {
        let config = AppConfig::parse(
            r#"
            [auth]
            api_keys = ["k1"]
            [[auth.methods]]
            mode = "oidc"
            provider = { type = "oidc", issuer = "https://login.example.com" }
            requirements = { audiences = ["support"], scopes = ["agent:call"] }
            "#,
        )
        .unwrap();
        assert_eq!(config.auth.api_keys, ["k1"]);
        let method = &config.auth.methods[0];
        assert_eq!(method.requirements.audiences, ["support"]);
        assert_eq!(method.requirements.scopes, ["agent:call"]);
        assert!(AppConfig::parse("[auth]\napi_key = \"k\"").is_err());
    }

    #[test]
    fn unknown_keys_are_rejected() {
        assert!(AppConfig::parse("sandbx = 1").is_err());
    }
}
