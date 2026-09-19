//! Creating an endpoint's Slack app without the operator visiting api.slack.com.
//!
//! Connecting an agent to Slack asks for four values copied out of Slack by
//! hand. Three of them can be obtained programmatically: `apps.manifest.create`
//! returns the signing secret and client credentials, and the OAuth install
//! returns the bot token and workspace id. `knowledge/integrations/slack-one-click-install.md`
//! records the live PoC that established this, including that the create call
//! accepts the manifest we already generate, whole.
//!
//! Why this is a seam rather than an implementation. Creating apps requires a
//! Slack *app configuration token*, which is an Everruns-the-company credential
//! — not something each self-hosted deployment can hold. So the OSS side owns
//! the manifest, the routes, and the credential storage, and a deployment that
//! has a token supplies the provisioner. With no provisioner installed the
//! one-click path reports itself unavailable and the copy-paste flow that
//! self-hosted already uses is untouched.

use async_trait::async_trait;

/// What `apps.manifest.create` hands back for a newly created Slack app.
///
/// `signing_secret` is one of the four values the operator used to copy by
/// hand; the client pair is what the subsequent OAuth install is performed
/// with. The `verification_token` Slack also returns is deliberately not
/// carried: it is the deprecated pre-signing-secret verification mechanism and
/// storing a credential nothing reads is a liability, not a convenience.
#[derive(Clone)]
pub struct SlackAppCredentials {
    /// Slack's id for the created app (`A…`), used to reap an orphan.
    pub app_id: String,
    pub client_id: String,
    pub client_secret: String,
    pub signing_secret: String,
}

/// Deliberately hand-written: the derived `Debug` would print three secrets,
/// and this type exists to be carried through handlers that log errors.
impl std::fmt::Debug for SlackAppCredentials {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SlackAppCredentials")
            .field("app_id", &self.app_id)
            .field("client_id", &self.client_id)
            .field("client_secret", &"<redacted>")
            .field("signing_secret", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SlackProvisioningError {
    /// This deployment holds no app configuration token. Not a failure: it is
    /// the self-hosted steady state, and the caller should point the operator
    /// at the manual flow rather than report an error.
    #[error("Slack app provisioning is not configured on this deployment")]
    Unavailable,
    /// Slack refused the call. The message is Slack's `error` code, which is a
    /// closed vocabulary (`ratelimited`, `invalid_manifest`, …) and safe to
    /// surface; it never carries our token.
    #[error("Slack rejected the request: {0}")]
    Rejected(String),
    /// The call did not complete — network, timeout, unparseable body.
    #[error("Could not reach Slack: {0}")]
    Unreachable(String),
}

pub type SlackProvisioningResult<T> = Result<T, SlackProvisioningError>;

/// Creates and reaps per-endpoint Slack apps on behalf of the deployment.
#[async_trait]
pub trait SlackAppProvisioner: Send + Sync {
    /// Create a Slack app from a manifest this server generated.
    async fn create_app(&self, manifest_yaml: &str)
    -> SlackProvisioningResult<SlackAppCredentials>;

    /// Delete an app created by `create_app`.
    ///
    /// Called when an install is abandoned before OAuth completes, so a
    /// half-finished flow does not leave an app in the deployment's Slack
    /// account that nothing references. Best-effort by contract: the caller
    /// logs a failure and moves on rather than trapping the endpoint in a
    /// state the UI cannot explain.
    async fn delete_app(&self, app_id: &str) -> SlackProvisioningResult<()>;

    fn name(&self) -> &'static str {
        "SlackAppProvisioner"
    }
}

/// The default on a deployment with no configuration token.
///
/// Reports `Unavailable` rather than being absent so the route can answer with
/// "use the manual flow" in one shape, whether the deployment installed no
/// provisioner at all or installed one that is not currently usable.
#[derive(Debug, Clone, Default)]
pub struct UnavailableSlackAppProvisioner;

#[async_trait]
impl SlackAppProvisioner for UnavailableSlackAppProvisioner {
    async fn create_app(
        &self,
        _manifest_yaml: &str,
    ) -> SlackProvisioningResult<SlackAppCredentials> {
        Err(SlackProvisioningError::Unavailable)
    }

    async fn delete_app(&self, _app_id: &str) -> SlackProvisioningResult<()> {
        Err(SlackProvisioningError::Unavailable)
    }

    fn name(&self) -> &'static str {
        "UnavailableSlackAppProvisioner"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_debug_redacts_every_secret() {
        let rendered = format!(
            "{:?}",
            SlackAppCredentials {
                app_id: "A0123".to_string(),
                client_id: "4567.89".to_string(),
                client_secret: "client-secret-value".to_string(),
                signing_secret: "signing-secret-value".to_string(),
            }
        );
        assert!(rendered.contains("A0123"), "{rendered}");
        assert!(rendered.contains("4567.89"), "{rendered}");
        assert!(!rendered.contains("client-secret-value"), "{rendered}");
        assert!(!rendered.contains("signing-secret-value"), "{rendered}");
    }

    #[tokio::test]
    async fn absent_provisioner_reports_unavailable_rather_than_failing() {
        let provisioner = UnavailableSlackAppProvisioner;
        assert!(matches!(
            provisioner.create_app("_meta: {}").await,
            Err(SlackProvisioningError::Unavailable)
        ));
    }
}
