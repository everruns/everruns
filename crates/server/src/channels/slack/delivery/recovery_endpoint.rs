//! Recover from immutable endpoint ownership; archival sessions keep their
//! pre-FK compatibility path. Mutable Slack routing tags grant no authority.
use crate::domains::agent_channels::record::slack_channel::SlackChannelConfig;
use crate::storage::{EncryptionService, SessionRow, StorageBackend};
use std::sync::Arc;

pub(super) async fn configuration(
    db: &Arc<StorageBackend>,
    encryption: Option<&Arc<EncryptionService>>,
    session: &SessionRow,
) -> anyhow::Result<Option<SlackChannelConfig>> {
    if session.channel_id.is_some() {
        let invoker = crate::channels::slack::actions::DbSlackActionInvoker::new(
            db.clone(),
            encryption.cloned(),
            session.org_id,
            session.id,
        );
        // THREAT[TM-SLACK-005]: reuse native action authorization, including
        // org, agent, endpoint FK, and current endpoint/agent liveness checks.
        let (_, endpoint) = invoker.resolve_action_channel(session).await?;
        return Ok(if endpoint.enabled && endpoint.status.is_live() {
            endpoint.slack_config()
        } else {
            None
        });
    }
    // Archival turns created before endpoint FKs still recover through their
    // org-scoped App FK, never a caller-editable app tag.
    let Some(app_id) = session.app_id else {
        return Ok(None);
    };
    let app =
        crate::domains::apps::queries::get_by_internal_id(db, encryption, session.org_id, app_id)
            .await?;
    Ok(app.and_then(|app| {
        app.slack_channel()
            .and_then(|endpoint| endpoint.slack_config())
    }))
}

/// Native recovery must not trust routing metadata on an ordinary API input.
pub(super) async fn trusted_native_route(
    db: &Arc<StorageBackend>,
    encryption: Option<&Arc<EncryptionService>>,
    session: &SessionRow,
    input_message_id: &str,
) -> anyhow::Result<(String, String)> {
    let invoker = crate::channels::slack::actions::DbSlackActionInvoker::new(
        db.clone(),
        encryption.cloned(),
        session.org_id,
        session.id,
    );
    Ok(invoker.trusted_post_route(input_message_id).await?)
}
