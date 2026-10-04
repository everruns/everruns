//! Pre-tool policy for client-side calls.
//!
//! Server tools run this chain inside `execute_single_tool`. Client-side calls
//! never reach that path: they are handed to the integrating client. The same
//! hooks have to run first, or a server policy that would block a call still
//! emits `tool.call_requested`.

use std::collections::HashSet;
use std::sync::Arc;

use crate::engine::event_emitter::EventEmitter;
use crate::engine::events::{EventContext, EventRequest, ToolCompletedData};
use crate::engine::message::ContentPart;
use crate::engine::phase_effects::PhaseEffectSink;
use crate::engine::tool_context::ToolContext;
use crate::engine::tool_execution::ToolExecutor;
use crate::engine::tool_fingerprint::{tool_call_fingerprint, tool_result_fingerprint};
use crate::engine::tool_narration::ToolNarrationPhase;
use crate::engine::tool_types::{ToolCall, ToolDefinition, ToolResult};

use super::{ActAtom, ExecutionContext, ToolCallResult};

/// Run the configured pre-tool chain on client-side calls.
///
/// Returns the calls a client may execute (with hook-transformed arguments),
/// their definitions, and the calls the chain blocked or deferred. Settled
/// calls are ordinary tool results and are omitted from `tool.call_requested`.
///
/// Calls with no definition (a provider's remote approval request) are not
/// model client capabilities and stay on the existing emit path.
pub(super) async fn apply_pre_tool_policy<T, E>(
    atom: &ActAtom<T, E>,
    context: &ExecutionContext,
    calls: Vec<ToolCall>,
    tool_definitions: &[ToolDefinition],
    network_access: Option<&crate::engine::network_access::NetworkAccessList>,
    locale: Option<&str>,
) -> (Vec<ToolCall>, Vec<ToolDefinition>, Vec<ToolCallResult>)
where
    T: ToolExecutor + Send + Sync + 'static,
    E: EventEmitter + Send + Sync + 'static,
{
    if calls.is_empty() || atom.pre_tool_hooks.is_empty() {
        let definitions = client_definitions(&calls, tool_definitions);
        return (calls, definitions, Vec::new());
    }

    // THREAT[TM-CLIENT-005]: decide every client call before any execution
    // request. A block or defer never becomes `tool.call_requested`.
    let visible_tool_names = Arc::new(
        tool_definitions
            .iter()
            .map(|def| def.name().to_string())
            .collect::<HashSet<_>>(),
    );
    let event_context = EventContext::from_execution_context(context);
    let mut approved = Vec::new();
    let mut settled = Vec::new();

    for call in calls {
        let Some(tool_def) = tool_definitions.iter().find(|def| def.name() == call.name) else {
            approved.push(call);
            continue;
        };

        let (tool_context, cancellation) = tool_context_for_call(
            atom,
            context,
            &event_context,
            &call.id,
            network_access,
            &visible_tool_names,
        );
        let _cancel_on_call_end = cancellation.drop_guard();
        let (updated, outcome) = super::act_hooks::pre_tool_use_outcome(
            &atom.pre_tool_hooks,
            call,
            tool_def,
            &tool_context,
        )
        .await;

        if let Some(mut tool_result) = outcome {
            super::act_hooks::run_post_tool_exec_hooks(
                &atom.post_tool_hooks,
                &atom.final_post_tool_hooks,
                &updated,
                tool_def,
                &mut tool_result,
                &tool_context,
            )
            .await;
            settled.push(
                record_settled_call(atom, context, &updated, tool_def, tool_result, locale).await,
            );
        } else {
            approved.push(updated);
        }
    }

    let definitions = client_definitions(&approved, tool_definitions);
    (approved, definitions, settled)
}

fn client_definitions(
    calls: &[ToolCall],
    tool_definitions: &[ToolDefinition],
) -> Vec<ToolDefinition> {
    if calls.is_empty() {
        return Vec::new();
    }
    tool_definitions
        .iter()
        .filter(|definition| match definition {
            ToolDefinition::ClientSide(tool) => calls.iter().any(|call| call.name == tool.name),
            _ => false,
        })
        .cloned()
        .collect()
}

/// Same per-call context server execution builds, so approval and guardrail
/// hooks see session services whether or not this process runs the tool.
pub(super) fn tool_context_for_call<T, E>(
    atom: &ActAtom<T, E>,
    context: &ExecutionContext,
    event_context: &EventContext,
    tool_call_id: &str,
    network_access: Option<&crate::engine::network_access::NetworkAccessList>,
    visible_tool_names: &Arc<HashSet<String>>,
) -> (ToolContext, tokio_util::sync::CancellationToken)
where
    T: ToolExecutor,
    E: PhaseEffectSink + 'static,
{
    let mut tool_context = ToolContext::from_services(context.session_id, &atom.context_services);
    if let Some(resolver) = tool_context.connection_resolver.as_ref()
        && let Some(bound) = resolver.for_execution(context.input_message_id.uuid())
    {
        tool_context.connection_resolver = Some(bound);
    }
    if let Some(scope) = tool_context.extension::<crate::tool_context::ExecutionServicesExt>() {
        scope
            .0
            .bind(&mut tool_context, context.input_message_id.uuid());
    }
    if let Some(invoker) = tool_context.mcp_invoker.as_ref()
        && let Some(bound) = invoker.for_execution(context.input_message_id.uuid())
    {
        tool_context.mcp_invoker = Some(bound);
    }
    if let Some(authority) = tool_context.session_creation_authority.as_ref()
        && let Some(bound) = authority.for_execution(context.input_message_id.uuid())
    {
        tool_context.session_creation_authority = Some(bound);
    }
    // Key file I/O by the attached workspace when known: pin the file store
    // to the workspace so shared-workspace sessions address the workspace's
    // files, not the session's own keyspace. For the default 1:1 case this
    // is a transparent pass-through.
    if let Some(workspace_id) = context.workspace_id {
        tool_context.workspace_id = workspace_id;
        if let Some(store) = tool_context.file_store.take() {
            tool_context.file_store = Some(
                crate::engine::session_files::WorkspaceScopedFileSystem::wrap(store, workspace_id),
            );
        }
    }
    // Resolve model paths through the mount resolver (EVE-660): `/workspace`
    // is a mount + cwd, not a per-store prefix. Applied over the
    // workspace-keyed store so resolution sits above re-keying.
    if let Some(store) = tool_context.file_store.take() {
        tool_context.file_store = Some(crate::engine::mount_fs::MountFs::wrap_if_needed(store));
    }
    tool_context.visible_tool_names = Some(visible_tool_names.clone());
    tool_context.network_access = network_access
        .cloned()
        .or_else(|| atom.context_services.network_access.clone());
    if tool_context.event_emitter.is_none() {
        tool_context.event_emitter =
            Some(Arc::new(atom.event_emitter.clone()) as Arc<dyn EventEmitter>);
    }
    tool_context.bind_to_turn(event_context.clone());
    tool_context.tool_call_id = Some(tool_call_id.to_string());

    let cancellation = tokio_util::sync::CancellationToken::new();
    tool_context.cancellation = Some(cancellation.clone());
    (tool_context, cancellation)
}

async fn record_settled_call<T, E>(
    atom: &ActAtom<T, E>,
    context: &ExecutionContext,
    tool_call: &ToolCall,
    tool_def: &ToolDefinition,
    tool_result: ToolResult,
    locale: Option<&str>,
) -> ToolCallResult
where
    T: ToolExecutor + Send + Sync + 'static,
    E: EventEmitter + Send + Sync + 'static,
{
    let event_context = EventContext::from_execution_context(context);
    let fingerprint = tool_call_fingerprint(tool_call);
    let display_name = crate::engine::localization::localized_tool_display_name(
        &tool_call.name,
        tool_def.display_name(),
        locale,
    );
    let capability_attribution = tool_def
        .capability_attribution()
        .map(|(id, name)| (id.to_string(), name.map(str::to_string)));
    // No tool.started. The call is not running, and a started event is the
    // shape an integrating client could mistake for permission to execute.
    let success = tool_result.error.is_none();
    let status = if success { "success" } else { "error" };
    let result_fingerprint = tool_result_fingerprint(&tool_call.name, &tool_result);
    let completed_phase = if success {
        ToolNarrationPhase::Completed
    } else {
        ToolNarrationPhase::Failed
    };
    let narration =
        atom.render_tool_narration(context, Some(tool_def), tool_call, completed_phase, locale);
    let completed = if success {
        let mut content = tool_result
            .result
            .as_ref()
            .map(|value| vec![ContentPart::tool_result_text(value)])
            .unwrap_or_default();
        if let Some(images) = &tool_result.images {
            for image in images {
                content.push(ContentPart::Image(
                    crate::engine::message::ImageContentPart::from_base64(
                        &image.base64,
                        &image.media_type,
                    ),
                ));
            }
        }
        ToolCompletedData::success(tool_call.id.clone(), tool_call.name.clone(), content, None)
    } else {
        ToolCompletedData::failure(
            tool_call.id.clone(),
            tool_call.name.clone(),
            status.to_string(),
            tool_result.error.clone().unwrap_or_default(),
            None,
        )
    }
    .with_fingerprints(fingerprint, result_fingerprint)
    .with_display_name(display_name)
    .with_capability_attribution(
        capability_attribution.as_ref().map(|(id, _)| id.clone()),
        capability_attribution
            .as_ref()
            .and_then(|(_, name)| name.clone()),
    )
    .with_narration(Some(narration));

    if let Err(error) = atom
        .event_emitter
        .emit(EventRequest::new(
            context.session_id,
            event_context,
            completed,
        ))
        .await
    {
        tracing::warn!(
            session_id = %context.session_id,
            tool_call_id = %tool_call.id,
            error = %error,
            "ActAtom: failed to emit tool.completed event"
        );
    }

    let connection_required = tool_result.connection_required.clone();
    ToolCallResult {
        tool_call: tool_call.clone(),
        result: tool_result,
        success,
        status: status.to_string(),
        connection_required,
        determinism_fatal: None,
    }
}
