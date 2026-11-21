//! Starting a session's sandbox as a copy of its source session's sandbox
//! (session fork, or `spawn_agent` with `sandbox: "fork"`).

use super::*;

/// Copy the sandbox of the session this one was forked or spawned from, when
/// a copy was requested and the provider can make one.
///
/// Returns the copied instance and the source's init completion, or `None`
/// for the fresh-sandbox path. Never fails: a missing source, a provider
/// without a copy primitive, or a provider error all mean "start fresh".
pub(super) async fn fork_from_source(
    context: &ToolContext,
    provider: &dyn SessionSandboxProvider,
    config: &SessionSandboxConfig,
) -> Option<(SessionSandboxInstance, Option<String>)> {
    let store = context
        .extensions
        .get::<crate::sandbox_state::SandboxStateStoreExt>()?;
    let source_session_id = match store.0.take_fork_source(context.session_id).await {
        Ok(Some(source)) => source,
        Ok(None) => return None,
        Err(error) => {
            tracing::warn!(%error, "could not read the sandbox copy request; starting fresh");
            return None;
        }
    };
    let source = match store
        .0
        .load_state(source_session_id, &config.provider)
        .await
    {
        Ok(Some(source)) => source,
        Ok(None) => {
            tracing::info!(
                source_session_id = %source_session_id,
                "source session has no sandbox on this provider to copy; starting fresh"
            );
            return None;
        }
        Err(error) => {
            tracing::warn!(%error, "could not load the source sandbox; starting fresh");
            return None;
        }
    };
    if source.status != SessionSandboxStatus::Running {
        tracing::info!(
            source_session_id = %source_session_id,
            status = ?source.status,
            "source sandbox is not running; starting fresh"
        );
        return None;
    }
    match provider.fork(context, config, &source.instance).await {
        Ok(Some(mut instance)) => {
            if let Some(metadata) = instance.metadata.as_object_mut() {
                metadata.insert(
                    "forked_from".to_string(),
                    serde_json::json!({
                        "session_id": source_session_id.to_string(),
                        "external_id": source.instance.external_id,
                    }),
                );
            }
            Some((instance, source.init_completed_at))
        }
        Ok(None) => {
            tracing::info!(
                provider = %config.provider,
                "provider cannot copy this sandbox; starting fresh"
            );
            None
        }
        Err(error) => {
            tracing::warn!(
                provider = %config.provider,
                error = ?error,
                "sandbox copy failed; starting fresh"
            );
            None
        }
    }
}
