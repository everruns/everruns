// PostgreSQL repository: the Slack channels a Socket Mode connection may serve.
//
// Decision: Socket Mode lives in the encrypted channel config, so this lists
// every enabled Slack channel and the caller decrypts and filters. The list is
// one row per Slack channel and is read every few seconds by the socket
// supervisor (`channels::slack::socket_mode`), which is cheap at any realistic
// number of Slack apps.

use super::super::super::IngressChannelRow;
use super::Database;
use anyhow::Result;

impl Database {
    /// Enabled Slack channels with their agent, as ingress rows.
    pub async fn list_enabled_slack_ingress_channels(&self) -> Result<Vec<IngressChannelRow>> {
        sqlx::query_as::<_, IngressChannelRow>(
            r#"
            SELECT
                ae.id AS channel_id,
                ae.public_id AS channel_public_id,
                ae.app_id AS legacy_app_id,
                ae.legacy_alias_id,
                agent.org_id,
                ae.agent_id,
                agent.public_id AS agent_public_id,
                COALESCE(NULLIF(agent.display_name, ''), agent.name) AS agent_name,
                agent.description AS agent_description,
                agent.harness_id,
                agent.status AS agent_status,
                agent.exposures_suspended,
                ae.virtual_user_id,
                ae.owner_principal_id,
                ae.resolved_owner_user_id,
                ae.channel_type,
                ae.channel_config,
                ae.channel_config_encrypted,
                ae.auth,
                ae.auth_encrypted,
                ae.enabled,
                ae.status AS channel_status,
                ae.created_at,
                ae.updated_at
            FROM agent_channels AS ae
            JOIN agents AS agent ON agent.id = ae.agent_id
            WHERE ae.channel_type = 'slack'
              AND ae.enabled = true
            ORDER BY ae.created_at, ae.id
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(Into::into)
    }
}
