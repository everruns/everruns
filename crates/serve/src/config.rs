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

    #[test]
    fn parses_full_config() {
        let config = AppConfig::parse(
            r#"
            name = "revenue-analyst"
            [sandbox]
            kind = "bashkit"
            [secrets]
            required = ["WAREHOUSE_URL"]
            [deploy]
            target = "everruns-cloud"
            "#,
        )
        .unwrap();
        assert_eq!(config.name.as_deref(), Some("revenue-analyst"));
        assert_eq!(config.sandbox.kind, SandboxKind::Bashkit);
        assert_eq!(config.secrets.required, vec!["WAREHOUSE_URL"]);
    }

    #[test]
    fn empty_config_defaults_to_no_sandbox() {
        let config = AppConfig::parse("").unwrap();
        assert_eq!(config.sandbox.kind, SandboxKind::None);
    }

    #[test]
    fn unknown_keys_are_rejected() {
        assert!(AppConfig::parse("sandbx = 1").is_err());
    }
}
