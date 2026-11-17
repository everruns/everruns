//! Durable checkpoints of a managed sandbox's workspace, and the hook a host
//! replaces them with (`workspace_sync`): a host whose own workspace is the
//! source of truth syncs instead of archiving.

use serde_json::Value;

use everruns_core::tool_context::ToolContext;
use everruns_core::tools::ToolExecutionResult;

use super::{
    SessionSandboxConfig, SessionSandboxProvider, SessionSandboxState, now_rfc3339,
    save_session_sandbox_state,
};

/// Provider-neutral key a provider sets on `SessionSandboxInstance::metadata`
/// to name the workspace revision its checkpoint produced.
const CHECKPOINT_REVISION_METADATA_KEY: &str = "checkpoint_revision";

/// Persist the provider checkpoint binding before the mutating tool result is
/// returned to the runtime.
///
/// Writes happen in this order (EVE-870):
///
/// 1. the provider uploads the archive and names a revision;
/// 2. the revision is recorded as an *unattached* checkpoint;
/// 3. the current environment binding is written, which makes the revision
///    recoverable (a hosted row, or the compatibility secret fallback);
/// 4. the checkpoint is attached, marking it authoritative.
///
/// A crash between 2 and 4 therefore leaves a checkpoint row nothing points at,
/// which garbage collection may drop, rather than an authoritative pointer to a
/// revision no committed turn produced.
pub async fn checkpoint_session_sandbox(
    context: &ToolContext,
    provider: &dyn SessionSandboxProvider,
    config: &SessionSandboxConfig,
    state: &mut SessionSandboxState,
) -> Result<(), ToolExecutionResult> {
    // A syncing host's workspace takes the change instead of an archive.
    if let Some(sync) = super::workspace_sync::installed(context) {
        return sync.pull(context, provider, config, state).await;
    }
    let checkpointed = provider
        .checkpoint(context, config, &state.instance)
        .await?;
    if checkpointed != state.instance {
        let revision = checkpointed
            .metadata
            .get(CHECKPOINT_REVISION_METADATA_KEY)
            .and_then(Value::as_str)
            .map(str::to_string);

        let recorded = match revision {
            Some(revision) => record_sandbox_checkpoint(context, config, &revision).await,
            None => None,
        };

        state.instance = checkpointed;
        state.updated_at = now_rfc3339();
        save_session_sandbox_state(context, state).await?;

        if let Some((store, sandbox, checkpoint_id)) = recorded {
            // A failure to attach must not fail the tool: the provider binding
            // has already advanced. The record remains collectable and the
            // error is surfaced operationally without falsely making it the
            // authoritative checkpoint.
            if let Err(error) = store
                .attach_checkpoint(sandbox.id, checkpoint_id, sandbox.generation)
                .await
            {
                tracing::warn!(
                    sandbox_id = %sandbox.id,
                    %error,
                    "failed to attach sandbox checkpoint; leaving it collectable"
                );
            }
        }
    }
    Ok(())
}

/// Drop the authoritative checkpoint when the tool call it was taken for never
/// committed, and rewind the provider's recovery state to the one before it.
///
/// `checkpoint_session_sandbox` attaches a checkpoint before the runtime settles
/// the tool result it belongs to. A control-plane crash in that window leaves
/// the workspace one revision ahead of the conversation: the agent resumes with
/// no record of the command, re-runs it, and finds a workspace the first attempt
/// already mutated. This runs on resume and puts the two back in step.
///
/// Deliberately conservative. It only ever rejects the *current* checkpoint, and
/// only when durable storage positively says the source call is not settled.
/// Anything it cannot establish — no store installed, no source call recorded,
/// a stale generation — leaves the pointer alone, because the failure mode of
/// over-eager rollback is discarding real work.
pub(crate) async fn reconcile_session_sandbox_checkpoint(
    context: &ToolContext,
    provider: &dyn SessionSandboxProvider,
    config: &SessionSandboxConfig,
    state: &mut SessionSandboxState,
) -> Result<(), ToolExecutionResult> {
    use crate::sandbox_checkpoint::{DurableToolResultStoreExt, SandboxCheckpointStoreExt};
    use everruns_core::durability::DurableToolCallStatus;

    // Both halves are needed to decide anything. Portable Framework hosts that
    // install neither keep the pre-EVE-870 behaviour.
    let Some(checkpoints) = context.extensions.get::<SandboxCheckpointStoreExt>() else {
        return Ok(());
    };
    let Some(durable) = context.extensions.get::<DurableToolResultStoreExt>() else {
        return Ok(());
    };
    let (checkpoints, durable) = (checkpoints.0.clone(), durable.0.clone());

    let sandbox = match checkpoints
        .ensure_sandbox(context.session_id, &config.provider)
        .await
    {
        Ok(sandbox) => sandbox,
        Err(error) => {
            tracing::warn!(%error, "failed to resolve logical sandbox; skipping reconciliation");
            return Ok(());
        }
    };

    let head = match checkpoints.current_checkpoint(sandbox.id).await {
        Ok(Some(head)) => head,
        Ok(None) => return Ok(()),
        Err(error) => {
            tracing::warn!(%error, "failed to read current checkpoint; skipping reconciliation");
            return Ok(());
        }
    };

    // A checkpoint with no source call was not taken for a tool result, so there
    // is nothing to compare it against and no reason to distrust it.
    let (Some(turn_id), Some(tool_call_id)) = (
        head.source_turn_id.as_deref(),
        head.source_tool_call_id.as_deref(),
    ) else {
        return Ok(());
    };

    let status = match durable.get_tool_call_status(turn_id, tool_call_id).await {
        Ok(status) => status,
        Err(error) => {
            tracing::warn!(%error, "failed to read durable tool call status; skipping reconciliation");
            return Ok(());
        }
    };

    // `Settled` is the only state that means "the conversation records a result
    // for this revision". `Interrupted` records that the call did *not* produce
    // one, and the transcript's interrupted placeholder invites the agent to
    // retry — against the workspace as it was before, which is the whole point.
    if matches!(status, Some(DurableToolCallStatus::Settled { .. })) {
        return Ok(());
    }

    let restored = match checkpoints
        .rollback_current_checkpoint(sandbox.id, head.id, sandbox.generation)
        .await
    {
        Ok(restored) => restored,
        Err(error) => {
            tracing::warn!(
                sandbox_id = %sandbox.id,
                %error,
                "failed to roll back uncommitted sandbox checkpoint"
            );
            return Ok(());
        }
    };

    tracing::info!(
        sandbox_id = %sandbox.id,
        rejected_revision = %head.workspace_revision,
        restored_revision = restored.as_ref().map(|c| c.workspace_revision.as_str()).unwrap_or("<none>"),
        "rolled sandbox back to its last committed checkpoint"
    );

    let rewound = provider
        .rewind_checkpoint(
            context,
            config,
            &state.instance,
            restored.as_ref().map(|c| c.workspace_revision.as_str()),
        )
        .await?;
    if rewound != state.instance {
        state.instance = rewound;
        state.updated_at = now_rfc3339();
        save_session_sandbox_state(context, state).await?;
    }

    Ok(())
}

/// Record an uploaded revision as an unattached checkpoint.
///
/// Returns the store, sandbox and checkpoint id so the caller can attach it
/// once the pointer write has succeeded. Portable Framework hosts without the
/// store installed get `None` and keep the secret-only behaviour.
async fn record_sandbox_checkpoint(
    context: &ToolContext,
    config: &SessionSandboxConfig,
    revision: &str,
) -> Option<(
    std::sync::Arc<dyn crate::sandbox_checkpoint::SandboxCheckpointStore>,
    crate::sandbox_checkpoint::SandboxRef,
    uuid::Uuid,
)> {
    use crate::sandbox_checkpoint::{
        NewSandboxCheckpoint, SandboxCheckpointKind, SandboxCheckpointStoreExt,
    };

    let store = context
        .extensions
        .get::<SandboxCheckpointStoreExt>()?
        .0
        .clone();

    let sandbox = match store
        .ensure_sandbox(context.session_id, &config.provider)
        .await
    {
        Ok(sandbox) => sandbox,
        Err(error) => {
            tracing::warn!(%error, "failed to resolve logical sandbox; skipping checkpoint record");
            return None;
        }
    };

    let checkpoint = match store
        .record_checkpoint(NewSandboxCheckpoint {
            sandbox_id: sandbox.id,
            generation: sandbox.generation,
            source_turn_id: context
                .event_context
                .as_ref()
                .and_then(|events| events.turn_id.as_ref())
                .map(ToString::to_string),
            source_tool_call_id: context.tool_call_id.clone(),
            kind: SandboxCheckpointKind::PortableWorkspace,
            provider_ref: None,
            workspace_revision: revision.to_string(),
        })
        .await
    {
        Ok(checkpoint) => checkpoint,
        Err(error) => {
            tracing::warn!(%error, "failed to record sandbox checkpoint");
            return None;
        }
    };

    Some((store, sandbox, checkpoint.id))
}
