//! The managed assistant's default is a shipped asset, never a provider call.
//! Render once per process, and attach only when an organization has no avatar.

use crate::domains::agents::avatar::{RenderedAvatarVariant, render_avatar};
use crate::storage::{AgentAvatarVariantInput, SetAgentAvatar, StorageBackend};
use everruns_contracts::typed_id::AgentId;
use std::sync::LazyLock;

static VARIANTS: LazyLock<Result<Vec<RenderedAvatarVariant>, String>> = LazyLock::new(|| {
    render_avatar(
        include_bytes!("../../assets/platform-chat-caretaker.png"),
        "image/png",
    )
    .map_err(|error| error.to_string())
});

pub(super) async fn ensure(
    db: &StorageBackend,
    org_id: i64,
    agent_id: AgentId,
) -> anyhow::Result<()> {
    if db
        .get_agent_avatar_source(org_id, agent_id.uuid())
        .await?
        .is_some()
    {
        return Ok(());
    }
    let variants = tokio::task::spawn_blocking(|| VARIANTS.as_ref().cloned().map_err(Clone::clone))
        .await?
        .map_err(anyhow::Error::msg)?;
    db.set_default_agent_avatar(SetAgentAvatar {
        org_id,
        agent_id: agent_id.uuid(),
        source: "preset:platform-caretaker".into(),
        variants: variants
            .into_iter()
            .map(|v| AgentAvatarVariantInput {
                variant: v.variant,
                content_type: v.content_type.into(),
                data: v.data,
            })
            .collect(),
    })
    .await?
    .ok_or_else(|| anyhow::anyhow!("Platform Chat agent disappeared during avatar provisioning"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::DEFAULT_ORG_ID;
    use uuid::Uuid;

    #[tokio::test]
    async fn caretaker_provisioning_is_idempotent_and_preserves_existing_branding() {
        let db = StorageBackend::test_database();
        let agent = AgentId::from_uuid(db.create_test_agent(DEFAULT_ORG_ID, Uuid::now_v7()).await);
        let (first, second) = tokio::join!(
            ensure(&db, DEFAULT_ORG_ID, agent),
            ensure(&db, DEFAULT_ORG_ID, agent)
        );
        first.unwrap();
        second.unwrap();
        let original = db
            .get_agent(DEFAULT_ORG_ID, agent)
            .await
            .unwrap()
            .unwrap()
            .avatar_id
            .unwrap();
        ensure(&db, DEFAULT_ORG_ID, agent).await.unwrap();
        assert_eq!(
            db.get_agent(DEFAULT_ORG_ID, agent)
                .await
                .unwrap()
                .unwrap()
                .avatar_id,
            Some(original)
        );
        let custom = db
            .set_agent_avatar(SetAgentAvatar {
                org_id: DEFAULT_ORG_ID,
                agent_id: agent.uuid(),
                source: "upload".into(),
                variants: vec![],
            })
            .await
            .unwrap()
            .unwrap();
        ensure(&db, DEFAULT_ORG_ID, agent).await.unwrap();
        assert_eq!(
            db.get_agent(DEFAULT_ORG_ID, agent)
                .await
                .unwrap()
                .unwrap()
                .avatar_id,
            Some(custom)
        );
        assert_eq!(
            db.get_agent_avatar_source(DEFAULT_ORG_ID, agent.uuid())
                .await
                .unwrap()
                .as_deref(),
            Some("upload")
        );
    }
}
