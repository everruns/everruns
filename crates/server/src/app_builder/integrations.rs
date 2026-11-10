use super::*;

fn github_app_token_minter(
    auth_config: &auth::AuthConfig,
) -> Option<crate::storage::GitHubAppTokenMinter> {
    auth_config.github_connection.as_ref().map(|config| {
        crate::storage::GitHubAppTokenMinter::new(config.app_id.clone(), config.private_key.clone())
            .with_user_client(crate::github_user_token::GitHubUserClient::new(
                config.client_id.as_deref(),
                config.client_secret.as_deref(),
            ))
    })
}

pub(super) fn build_connection_resolver(
    db: &Arc<StorageBackend>,
    encryption: &Arc<EncryptionService>,
    auth_config: &auth::AuthConfig,
    egress: Arc<dyn everruns_core::EgressService>,
) -> Arc<dyn everruns_core::connection_services::UserConnectionResolver> {
    Arc::new(crate::storage::DbConnectionResolver::new(
        db.as_ref().clone(),
        encryption.as_ref().clone(),
        github_app_token_minter(auth_config),
        egress,
    ))
}

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

pub(super) fn optional_connection_resolver(
    db: &Arc<StorageBackend>,
    encryption: &Option<Arc<EncryptionService>>,
    auth_config: &auth::AuthConfig,
    egress: Arc<dyn everruns_core::EgressService>,
) -> Option<Arc<dyn everruns_core::connection_services::UserConnectionResolver>> {
    encryption
        .as_ref()
        .map(|enc| build_connection_resolver(db, enc, auth_config, egress))
}
