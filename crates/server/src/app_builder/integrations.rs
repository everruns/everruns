use super::*;

impl ServerAppBuilder {
    /// Replace the connector registry (user connections catalog, EVE-879).
    ///
    /// Defaults to the OSS preset (`crate::platform::oss_connector_registry`,
    /// inventory-discovered). Connectors are hosted control-plane composition,
    /// not part of the shared `HostComposition` runtime surface.
    pub fn connector_registry(
        mut self,
        registry: everruns_contracts::connector::ConnectorRegistry,
    ) -> Self {
        self.connector_registry = Some(registry);
        self
    }

    /// Replace the system email sender (EVE-879).
    ///
    /// Defaults to the environment-configured sender
    /// (`crate::platform::system_email_sender`; disabled when no provider is
    /// configured). Email delivery is hosted product composition, not part of
    /// the shared `HostComposition` runtime surface.
    pub fn email_sender(mut self, sender: Arc<dyn crate::records::email::EmailSender>) -> Self {
        self.email_sender = Some(sender);
        self
    }

    /// Supply a deployment-owned Slack app provisioner.
    pub fn slack_app_provisioner(
        mut self,
        provisioner: Arc<
            dyn crate::domains::agent_channels::record::slack_provisioning::SlackAppProvisioner,
        >,
    ) -> Self {
        self.slack_app_provisioner = Some(provisioner);
        self
    }
}
