//! AWS Bedrock provider configuration (requires the `bedrock` feature).
//!
//! [`Bedrock`] is the value-first provider configuration for Amazon Bedrock's
//! `ConverseStream` API. Two ways to authenticate:
//!
//! - [`Bedrock::new`] takes static AWS keys and a region. Deterministic: reads
//!   no environment.
//! - [`Bedrock::default_chain`] uses the AWS default credential chain: the
//!   standard `AWS_*` environment variables, the shared profile and SSO, web
//!   identity, ECS / Amazon Bedrock AgentCore Runtime container credentials,
//!   then the EC2 instance profile. A process running under an IAM role
//!   needs no keys at all.
//!
//! Pass either to [`AgentBuilder::provider`](crate::AgentBuilder::provider) or
//! [`Model::new`](crate::Model::new). The model id is the Bedrock model id or
//! inference profile, such as `us.anthropic.claude-haiku-4-5-20251001-v1:0`.
//!
//! Bedrock is deliberately not part of [`providers::from_env`](super::from_env):
//! the default chain always "succeeds" at construction time, so it would
//! shadow every provider tried after it.
//!
//! The `everruns_drivers::bedrock` driver is re-exported here ([`BedrockChatDriver`],
//! [`register_driver`]) for embedders who need the low-level driver directly.

use std::fmt;

use crate::Provider;

/// Re-exported `everruns_drivers::bedrock` driver for direct, low-level use.
pub use everruns_drivers::bedrock::{BedrockChatDriver, register_driver};

/// How a [`Bedrock`] configuration authenticates.
#[derive(Clone)]
enum Auth {
    Static {
        access_key_id: String,
        secret_access_key: String,
        session_token: Option<String>,
    },
    DefaultChain,
}

/// AWS Bedrock provider configuration.
///
/// Build one with [`Bedrock::new`] (static keys) or
/// [`Bedrock::default_chain`] (IAM role / ambient AWS credentials). Select the
/// model separately.
///
/// Keys and session tokens are redacted from [`Debug`] output.
#[derive(Clone)]
pub struct Bedrock {
    auth: Auth,
    region: Option<String>,
}

impl Bedrock {
    /// Configure Bedrock with static AWS keys.
    ///
    /// Deterministic: reads no environment. Add
    /// [`session_token`](Self::session_token) for temporary credentials.
    ///
    /// ```
    /// use everruns::{Model, providers::bedrock::Bedrock};
    ///
    /// let model = Model::new(
    ///     "us.anthropic.claude-haiku-4-5-20251001-v1:0",
    ///     Bedrock::new("AKIA...", "secret", "us-east-1"),
    /// );
    /// # let _ = model;
    /// ```
    pub fn new(
        access_key_id: impl Into<String>,
        secret_access_key: impl Into<String>,
        region: impl Into<String>,
    ) -> Self {
        Self {
            auth: Auth::Static {
                access_key_id: access_key_id.into(),
                secret_access_key: secret_access_key.into(),
                session_token: None,
            },
            region: Some(region.into()),
        }
    }

    /// Configure Bedrock on the AWS default credential chain.
    ///
    /// Credentials resolve lazily on the first request and refresh before
    /// they expire, so an ECS task, an EC2 instance profile, or an Amazon
    /// Bedrock AgentCore Runtime execution role works with no keys in the
    /// application. The role needs `bedrock:InvokeModelWithResponseStream` on
    /// the model or inference profile.
    ///
    /// Region is [`region`](Self::region) when set, else `AWS_REGION`, else
    /// `AWS_DEFAULT_REGION`, else `us-east-1`.
    ///
    /// For standalone hosts only: a multi-tenant server must not pick up its
    /// own ambient AWS identity on behalf of tenants.
    ///
    /// ```no_run
    /// use everruns::{Agent, providers::bedrock::Bedrock};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let agent = Agent::builder()
    ///     .instructions("You are concise.")
    ///     .provider(Bedrock::default_chain().region("us-west-2"))
    ///     .model("us.anthropic.claude-haiku-4-5-20251001-v1:0")
    ///     .build()?;
    /// # let _ = agent;
    /// # Ok(())
    /// # }
    /// ```
    pub fn default_chain() -> Self {
        Self {
            auth: Auth::DefaultChain,
            region: None,
        }
    }

    /// Set the AWS region, which selects the Bedrock endpoint.
    pub fn region(mut self, region: impl Into<String>) -> Self {
        self.region = Some(region.into());
        self
    }

    /// Attach the session token of temporary static credentials. Ignored by
    /// [`default_chain`](Self::default_chain), which resolves its own.
    pub fn session_token(mut self, session_token: impl Into<String>) -> Self {
        if let Auth::Static {
            session_token: token,
            ..
        } = &mut self.auth
        {
            *token = Some(session_token.into());
        }
        self
    }
}

impl fmt::Debug for Bedrock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = f.debug_struct("Bedrock");
        match &self.auth {
            Auth::Static { session_token, .. } => debug
                .field("auth", &"static")
                .field("access_key_id", &"[REDACTED]")
                .field("secret_access_key", &"[REDACTED]")
                .field(
                    "session_token",
                    &session_token.as_ref().map(|_| "[REDACTED]"),
                ),
            Auth::DefaultChain => debug.field("auth", &"default_chain"),
        };
        debug.field("region", &self.region).finish()
    }
}

impl From<Bedrock> for Provider {
    fn from(config: Bedrock) -> Self {
        let provider = match config.auth {
            Auth::Static {
                access_key_id,
                secret_access_key,
                session_token,
            } => {
                let region = config.region.unwrap_or_else(|| "us-east-1".to_string());
                let mut credential = everruns_drivers::bedrock::BedrockCredential::new(
                    access_key_id,
                    secret_access_key,
                    region,
                );
                if let Some(token) = session_token {
                    credential = credential.with_session_token(token);
                }
                everruns_drivers::bedrock::provider("bedrock", credential)
            }
            Auth::DefaultChain => {
                everruns_drivers::bedrock::provider_from_default_chain("bedrock", config.region)
            }
        };
        provider.with_driver_id(crate::DriverId::Bedrock)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_redacts_keys_and_token() {
        let rendered = format!(
            "{:?}",
            Bedrock::new("AKIA-MARKER", "SECRET-MARKER", "us-east-1").session_token("TOKEN-MARKER")
        );
        for secret in ["AKIA-MARKER", "SECRET-MARKER", "TOKEN-MARKER"] {
            assert!(!rendered.contains(secret), "{rendered}");
        }
        assert!(rendered.contains("[REDACTED]"), "{rendered}");
        assert!(rendered.contains("us-east-1"), "{rendered}");

        let rendered = format!("{:?}", Bedrock::default_chain().region("us-west-2"));
        assert!(rendered.contains("default_chain"), "{rendered}");
        assert!(rendered.contains("us-west-2"), "{rendered}");
    }
}
