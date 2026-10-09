use crate::domains::agent_channels::ingress::row_to_ingress;
use crate::records::SlackChannelConfig;
use crate::storage::{EncryptionService, HealthIssueRow, ObserveHealthIssue, StorageBackend};
use chrono::Utc;
use std::{sync::Arc, time::Duration};
use uuid::Uuid;

const API_BASE: &str = "https://slack.com/api";
const AUTH_FAILURES: &[&str] = &[
    "invalid_auth",
    "not_authed",
    "token_revoked",
    "token_expired",
    "account_inactive",
];

pub struct SlackHealthService {
    db: Arc<StorageBackend>,
    encryption: Option<Arc<EncryptionService>>,
    client: reqwest::Client,
    api_base: String,
}

impl SlackHealthService {
    pub fn new(db: Arc<StorageBackend>, encryption: Option<Arc<EncryptionService>>) -> Self {
        Self {
            db,
            encryption,
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("static Slack HTTP client"),
            api_base: API_BASE.into(),
        }
    }
    pub(crate) fn with_api_base(mut self, base: String) -> Self {
        self.api_base = base;
        self
    }

    pub async fn check(&self, public_id: &str) -> anyhow::Result<()> {
        self.check_inner(public_id, None).await.map(|_| ())
    }

    pub async fn check_requested(&self, public_id: &str, issue_id: Uuid) -> anyhow::Result<bool> {
        self.check_inner(public_id, Some(issue_id)).await
    }

    async fn check_inner(&self, public_id: &str, issue_id: Option<Uuid>) -> anyhow::Result<bool> {
        let Some(row) = self.db.get_ingress_channel_by_public_id(public_id).await? else {
            return Ok(true);
        };
        let _lock = self.db.lock_slack_install(row.channel_id).await?;
        let Some(row) = self.db.get_ingress_channel_by_public_id(public_id).await? else {
            return Ok(true);
        };
        // Serialize cooldown with checks and installation replacement across replicas.
        if let Some(id) = issue_id
            && let Some(issue) = self.db.get_health_issue(row.org_id, id).await?
            && Utc::now() - issue.last_checked_at < chrono::Duration::seconds(15)
        {
            return Ok(false);
        }
        if row.channel_type != "slack" {
            return Ok(true);
        }
        let revision = row.updated_at;
        let channel_id = row.channel_id;
        let org_id = row.org_id;
        let applicable =
            row.enabled && row.channel_status != "disabled" && row.agent_status == "active";
        let (_, channel) = row_to_ingress(self.encryption.as_ref(), row)?;
        let config: SlackChannelConfig = serde_json::from_value(channel.channel_config)?;
        let checked_at = Utc::now();
        let (status, missing_scopes, error_code) = if !applicable || config.bot_token.is_empty() {
            ("inapplicable", Vec::new(), None)
        } else {
            match self.probe(&config).await {
                Ok((missing, error)) => (
                    if missing.is_empty() && error.is_none() {
                        "resolved"
                    } else {
                        "open"
                    },
                    missing,
                    error,
                ),
                Err(()) => (
                    "needs_check",
                    Vec::new(),
                    Some("verification_unavailable".into()),
                ),
            }
        };
        self.db
            .observe_health_issue(ObserveHealthIssue {
                org_id,
                channel_id,
                channel_revision: revision,
                status: status.into(),
                missing_scopes,
                error_code,
                checked_at,
            })
            .await?;
        Ok(true)
    }

    async fn probe(
        &self,
        config: &SlackChannelConfig,
    ) -> Result<(Vec<String>, Option<String>), ()> {
        let response = self
            .client
            .post(format!("{}/auth.test", self.api_base))
            .bearer_auth(&config.bot_token)
            .send()
            .await
            .map_err(|_| ())?;
        if !response.status().is_success() {
            return Err(());
        }
        let scopes = response
            .headers()
            .get("x-oauth-scopes")
            .and_then(|h| h.to_str().ok())
            .map(|s| {
                s.split(',')
                    .map(str::trim)
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            });
        let body: serde_json::Value = response.json().await.map_err(|_| ())?;
        if body["ok"] != true {
            return match body["error"]
                .as_str()
                .filter(|code| AUTH_FAILURES.contains(code))
            {
                Some(code) => Ok((Vec::new(), Some(code.into()))),
                None => Err(()),
            };
        }
        // THREAT[TM-SLACK-009]: a valid token in another workspace is not recovery.
        if config
            .team_id
            .as_deref()
            .is_some_and(|team| body["team_id"].as_str() != Some(team))
        {
            return Err(());
        }
        let scopes = scopes.ok_or(())?;
        let missing = crate::records::slack_channel::slack_bot_scopes(config.agent_surface_enabled)
            .into_iter()
            .filter(|required| !scopes.iter().any(|scope| scope == required))
            .map(str::to_string)
            .collect::<Vec<_>>();
        let error = (!missing.is_empty()).then(|| "missing_scope".into());
        Ok((missing, error))
    }

    pub fn spawn(self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            loop {
                let mut after = Uuid::nil();
                loop {
                    let channels = match self.db.health_sweep_channels(after, 50).await {
                        Ok(rows) => rows,
                        Err(error) => {
                            tracing::warn!(%error,"Health reconciliation failed");
                            break;
                        }
                    };
                    if channels.is_empty() {
                        break;
                    }
                    let previous = after;
                    for id in channels {
                        if let Ok(Some(row)) = self.db.get_ingress_channel_by_public_id(&id).await {
                            after = row.channel_id;
                        }
                        if let Err(error) = self.check(&id).await {
                            tracing::warn!(%error,"Slack health check failed");
                        }
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                    // Deletion or lookup failures must not trap a sweep on one page.
                    if after == previous {
                        break;
                    }
                }
                // Spread replicas' sweeps; unavailable/limited probes wait for the next sweep.
                let jitter = Uuid::new_v4().as_u128() % 31;
                tokio::time::sleep(Duration::from_secs(300 + jitter as u64)).await;
            }
        })
    }
}

pub fn issue_copy(row: &HealthIssueRow) -> (String, String) {
    if row.code == super::active_turns::ACTIVE_TURN_LIMIT {
        return super::active_turns::copy(&row.status);
    }
    if row.status == "needs_check" {
        return ("Slack permissions need a check".into(),"Everruns could not verify this installation. Check again before assuming it is healthy.".into());
    }
    if row.status == "inapplicable" {
        return (
            "Slack issue no longer applies".into(),
            "The affected integration is disabled or no longer configured.".into(),
        );
    }
    if row.status == "resolved" {
        return (
            "Slack permissions verified".into(),
            "This installation has the required permissions.".into(),
        );
    }
    if row.error_code.as_deref() == Some("verification_unavailable")
        && row.missing_scopes.is_empty()
    {
        return (
            "Slack installation needs attention".into(),
            "The previously detected issue is still unresolved. Reconnect Slack or check again to verify current credentials and permissions.".into(),
        );
    }
    if row
        .error_code
        .as_deref()
        .is_some_and(|code| AUTH_FAILURES.contains(&code))
    {
        return ("Slack credentials need to be renewed".into(),"Slack rejected this installation's credentials. Reconnect Slack or update its bot token.".into());
    }
    if row.missing_scopes == ["reactions:write"] {
        (
            "Slack reactions need an additional permission".into(),
            "This bot cannot add emoji reactions. Reconnect Slack to grant permission.".into(),
        )
    } else {
        ("Slack needs additional permissions".into(),"Some Slack features are unavailable. Review the missing permissions and reconnect this installation.".into())
    }
}
