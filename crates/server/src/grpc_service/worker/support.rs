//! Helpers shared by the `WorkerService` handlers in this module tree.
//!
//! These were file-local to `worker_service_impl.rs` before the handlers were
//! split out by domain; they are crate-visible now because every handler module
//! needs them, not because anything outside `grpc_service` should call them.

use crate::domains::common::CommandErrorKind;
use crate::grpc_service::*;

/// The task-notification stream, named so the inherent handler in
/// [`super::notifications`] and the trait's associated type are one definition
/// rather than two that must be kept in step.
pub(crate) type TaskNotificationStream =
    Pin<Box<dyn futures::Stream<Item = Result<TaskNotification, Status>> + Send>>;

pub(crate) const COMMAND_API_VERSION_V1: &str = "v1";
pub(crate) const MAX_EXECUTE_COMMAND_PARAMS_BYTES: usize = 1024 * 1024;
pub(crate) const DEFAULT_TURN_CONTEXT_MESSAGE_LIMIT: i32 = 200;
pub(crate) const MAX_TURN_CONTEXT_MESSAGE_LIMIT: i32 =
    crate::storage::repository::MESSAGE_SAFETY_LIMIT as i32;

/// Log an internal failure server-side and return a client-safe `Status`.
///
/// THREAT[TM-API-005]: worker gRPC `Status` messages cross the server/worker trust
/// boundary, and `grpc_status_to_error` in `crates/worker` copies `status.message()`
/// verbatim into runtime errors — from there they reach worker logs, durable workflow
/// failure records, and session error surfaces. Only the fixed `context` string may
/// travel; sqlx text, Postgres index names, and source paths stay in the server log.
///
/// `internal_statuses_never_carry_source_errors` below pins that every internal
/// failure in this file goes through here or an equally generic literal.
pub(crate) fn internal_status(context: &'static str, error: impl std::fmt::Display) -> Status {
    tracing::error!(%error, "{}", context);
    Status::internal(context)
}

pub(crate) fn flatten_secret_bindings(
    bindings: std::collections::HashMap<String, Vec<everruns_mcp::McpSecretBinding>>,
) -> Vec<proto::McpSecretBinding> {
    bindings
        .into_iter()
        .flat_map(|(tool_name, bindings)| {
            bindings
                .into_iter()
                .map(move |binding| proto::McpSecretBinding {
                    tool_name: tool_name.clone(),
                    parameter_name: binding.parameter_name,
                    value: binding.value,
                    setup_url: binding.setup_url,
                    label: binding.label,
                })
        })
        .collect()
}

pub(crate) fn resolved_mcp_server_to_proto(
    resolved: crate::domains::mcp_servers::McpServerResolved,
    secret_bindings: std::collections::HashMap<String, Vec<everruns_mcp::McpSecretBinding>>,
) -> McpServerInfo {
    McpServerInfo {
        id: Some(proto::Uuid {
            value: resolved.id.to_string(),
        }),
        name: resolved.name,
        url: resolved.url,
        api_key: resolved.api_key,
        headers: resolved.headers,
        auth_mode: resolved.auth_mode.to_string(),
        protocol_mode: resolved.protocol_mode.to_string(),
        oauth_provider_id: resolved.oauth_provider_id,
        secret_bindings: flatten_secret_bindings(secret_bindings),
        acts_as: resolved.acts_as.to_string(),
        connection_subject_kind: None,
        connection_subject_name: None,
        connection_setup_url: None,
    }
}

pub(crate) fn apply_proto_secret_binding_schemas(
    definitions: &mut [McpToolDef],
    bindings: &[everruns_core::McpSecretBindingMetadata],
) {
    for binding in bindings {
        if !everruns_core::mcp_server::is_valid_mcp_server_name(&binding.server_name) {
            continue;
        }
        let tool_name = everruns_core::mcp_tool_name(&binding.server_name, &binding.tool_name);
        let Some(definition) = definitions
            .iter_mut()
            .find(|definition| definition.name == tool_name)
        else {
            continue;
        };
        if let Some(parameters) = definition.parameters.as_mut() {
            let mut json = everruns_internal_protocol::proto_struct_to_json(parameters);
            if let Some(object) = json.as_object_mut() {
                if let Some(properties) = object
                    .get_mut("properties")
                    .and_then(serde_json::Value::as_object_mut)
                {
                    properties.remove(&binding.parameter_name);
                }
                if let Some(required) = object
                    .get_mut("required")
                    .and_then(serde_json::Value::as_array_mut)
                {
                    required.retain(|value| value.as_str() != Some(&binding.parameter_name));
                }
            }
            *parameters = everruns_internal_protocol::json_to_proto_struct(&json);
        }
        let status = if binding.configured {
            "configured"
        } else {
            "setup required"
        };
        definition.description.push_str(&format!(
            "\n\nCredential '{}' is securely bound ({status}); do not request or supply it. Setup: {}",
            binding.parameter_name, binding.setup_url
        ));
    }
}

pub(crate) fn normalize_turn_context_message_limit(
    requested_limit: Option<i32>,
    default_limit: i32,
) -> i32 {
    let normalized_default = default_limit.clamp(1, MAX_TURN_CONTEXT_MESSAGE_LIMIT);
    requested_limit
        .filter(|&l| l > 0)
        .unwrap_or(normalized_default)
        .clamp(1, MAX_TURN_CONTEXT_MESSAGE_LIMIT)
}

pub(crate) fn command_error_kind(error: &crate::domains::common::CommandError) -> i32 {
    match &error.kind {
        CommandErrorKind::BadRequest(_) => 1,
        CommandErrorKind::Unprocessable(_) => 1,
        CommandErrorKind::Forbidden(_) => 2,
        CommandErrorKind::NotFound(message) if message.starts_with("Unknown command:") => 1,
        CommandErrorKind::NotFound(_) => 3,
        CommandErrorKind::Conflict(_) => 4,
        CommandErrorKind::RateLimited(_) => 1,
        CommandErrorKind::Internal(_) => 5,
    }
}

pub(crate) fn command_error_to_proto(
    error: crate::domains::common::CommandError,
) -> ProtoCommandError {
    let message = match &error.kind {
        CommandErrorKind::Internal(inner) => {
            // THREAT[TM-API-005]: ExecuteCommand callers receive no internal diagnostics.
            tracing::error!(error = %inner, "gRPC command failed");
            "Internal server error".to_string()
        }
        _ => error.to_string(),
    };

    ProtoCommandError {
        kind: command_error_kind(&error),
        message,
    }
}

pub(crate) fn command_error_to_status(error: crate::domains::common::CommandError) -> Status {
    match error.kind {
        CommandErrorKind::BadRequest(message) => Status::invalid_argument(message),
        CommandErrorKind::Unprocessable(message) => Status::failed_precondition(message),
        CommandErrorKind::Forbidden(message) => Status::permission_denied(message),
        CommandErrorKind::NotFound(message) => Status::not_found(message),
        CommandErrorKind::Conflict(message) => Status::failed_precondition(message),
        CommandErrorKind::RateLimited(message) => Status::resource_exhausted(message),
        // THREAT[TM-API-005]: mirror command_error_to_proto — the caller gets no diagnostics.
        CommandErrorKind::Internal(inner) => internal_status("Internal server error", inner),
    }
}

pub(crate) fn command_schema_hash(
    meta: &crate::domains::common::CommandMeta,
    positional_arg: Option<&str>,
) -> String {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(meta.name.as_bytes());
    hasher.update([0]);
    hasher.update(meta.category.as_bytes());
    hasher.update([0]);
    hasher.update(meta.description.as_bytes());
    hasher.update([0]);
    hasher.update(meta.method.as_bytes());
    hasher.update([0]);
    hasher.update(meta.path.as_bytes());
    hasher.update([0]);
    if let Some(positional_arg) = positional_arg {
        hasher.update(positional_arg.as_bytes());
    }
    hex::encode(hasher.finalize())
}

// Helper functions for status conversion

pub(crate) fn payment_error_to_status(error: everruns_provider::error::AgentLoopError) -> Status {
    use everruns_provider::error::AgentLoopError;

    match error {
        AgentLoopError::Configuration(message) | AgentLoopError::ToolExecution(message) => {
            Status::failed_precondition(message)
        }
        AgentLoopError::SessionNotFound(session_id) => {
            Status::not_found(format!("Session not found: {session_id}"))
        }
        AgentLoopError::MessageStore(error) => internal_status("Failed to store message", error),
        AgentLoopError::Internal(error) => internal_status("Internal server error", error),
        other => internal_status("Internal server error", other),
    }
}

pub(crate) fn circuit_state_to_proto(state: CircuitState) -> ProtoCircuitBreakerState {
    match state {
        CircuitState::Closed => ProtoCircuitBreakerState::Closed,
        CircuitState::Open => ProtoCircuitBreakerState::Open,
        CircuitState::HalfOpen => ProtoCircuitBreakerState::HalfOpen,
    }
}

pub(crate) fn workflow_status_to_proto(status: WorkflowStatus) -> DurableWorkflowStatus {
    match status {
        WorkflowStatus::Pending => DurableWorkflowStatus::Pending,
        WorkflowStatus::Running => DurableWorkflowStatus::Running,
        WorkflowStatus::Completed => DurableWorkflowStatus::Completed,
        WorkflowStatus::Failed => DurableWorkflowStatus::Failed,
        WorkflowStatus::Cancelled => DurableWorkflowStatus::Cancelled,
        WorkflowStatus::ContinuedAsNew => DurableWorkflowStatus::ContinuedAsNew,
    }
}

pub(crate) fn proto_to_workflow_status(status: DurableWorkflowStatus) -> WorkflowStatus {
    match status {
        DurableWorkflowStatus::Pending => WorkflowStatus::Pending,
        DurableWorkflowStatus::Running => WorkflowStatus::Running,
        DurableWorkflowStatus::Completed => WorkflowStatus::Completed,
        DurableWorkflowStatus::Failed => WorkflowStatus::Failed,
        DurableWorkflowStatus::Cancelled => WorkflowStatus::Cancelled,
        DurableWorkflowStatus::ContinuedAsNew => WorkflowStatus::ContinuedAsNew,
        DurableWorkflowStatus::Unspecified => WorkflowStatus::Pending,
    }
}

#[cfg(test)]
mod tests {

    /// Every `.rs` file under `dir`, recursively. The guard above covers a tree, so
    /// a handler module added tomorrow is covered without anyone remembering to list
    /// it here.
    fn collect_rust_sources(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let entries = std::fs::read_dir(dir).expect("read gRPC source directory");
        for entry in entries {
            let path = entry.expect("read directory entry").path();
            if path.is_dir() {
                collect_rust_sources(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                out.push(path);
            }
        }
    }
    use super::{
        DEFAULT_TURN_CONTEXT_MESSAGE_LIMIT, MAX_TURN_CONTEXT_MESSAGE_LIMIT, command_error_to_proto,
        internal_status, normalize_turn_context_message_limit, resolved_mcp_server_to_proto,
    };

    const RAW_STORAGE_ERROR: &str = "error returned from database: relation \"agents\" does not \
                                     exist at sqlx-postgres/src/connection.rs:666";

    #[test]
    fn internal_status_keeps_the_source_error_server_side() {
        let status = internal_status("Failed to get agent", RAW_STORAGE_ERROR);

        assert_eq!(status.code(), tonic::Code::Internal);
        assert_eq!(status.message(), "Failed to get agent");
        assert!(!status.message().contains("sqlx"));
        assert!(!status.message().contains("agents"));
    }

    /// THREAT[TM-API-005]: a new RPC that formats its source error into the Status
    /// re-opens the leak this surface was audited for, so pin the shape rather than
    /// the handful of call sites. The needles are assembled with `concat!` so this
    /// test's own source is not an offender.
    ///
    /// This walks the whole `grpc_service` tree rather than naming files. When the
    /// handlers were split out of `worker_service_impl.rs` the previous
    /// `include_str!` of that one file would still have compiled against the
    /// delegation layer left behind, passing while covering none of the handlers
    /// that actually build a `Status`. A guard that enumerates its own inputs is a
    /// guard a refactor can walk out from under.
    #[test]
    fn internal_statuses_never_carry_source_errors() {
        let call = concat!("Status", "::internal(");
        let literal = format!("{call}\"");
        let helper = format!("{call}context)");

        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/grpc_service");
        let mut sources: Vec<std::path::PathBuf> = Vec::new();
        collect_rust_sources(&root, &mut sources);
        assert!(
            sources.len() > 1,
            "found {} source file(s) under {}; the guard is not reading the tree it \
             is supposed to cover",
            sources.len(),
            root.display()
        );

        let mut offenders: Vec<String> = Vec::new();
        for path in &sources {
            let text = std::fs::read_to_string(path).expect("read gRPC source");
            for (index, line) in text.lines().enumerate() {
                if line.contains(call) && !line.contains(&literal) && !line.contains(&helper) {
                    let name = path.strip_prefix(&root).unwrap_or(path);
                    offenders.push(format!("{}:{}: {}", name.display(), index + 1, line.trim()));
                }
            }
        }

        assert!(
            offenders.is_empty(),
            "internal Status messages must be fixed literals or go through \
             internal_status(), which logs the source error instead of sending it:\n{}",
            offenders.join("\n")
        );
    }

    #[test]
    fn command_error_proto_redacts_internal_details() {
        let raw = "postgres query failed at sqlx-postgres/src/connection.rs:666";

        let proto = command_error_to_proto(crate::domains::common::CommandError::internal(
            anyhow::anyhow!(raw),
        ));
        assert_eq!(proto.message, "Internal server error");
        assert!(!proto.message.contains(raw));
    }

    #[test]
    fn command_error_proto_redacts_unique_conflict_details() {
        let raw = "error returned from database: duplicate key value violates unique constraint \
                   \"idx_memories_org_name_active\" at sqlx-postgres/src/connection.rs:666";
        let error = crate::domains::common::classify_anyhow(anyhow::anyhow!(raw));
        let proto = command_error_to_proto(error);

        assert_eq!(proto.kind, 4);
        assert_eq!(proto.message, crate::errors::ALREADY_EXISTS_DETAIL);
        assert!(!proto.message.contains(raw));
    }

    #[test]
    fn normalize_turn_context_message_limit_uses_clamped_default() {
        assert_eq!(
            normalize_turn_context_message_limit(None, DEFAULT_TURN_CONTEXT_MESSAGE_LIMIT),
            DEFAULT_TURN_CONTEXT_MESSAGE_LIMIT
        );
        assert_eq!(
            normalize_turn_context_message_limit(None, i32::MAX),
            MAX_TURN_CONTEXT_MESSAGE_LIMIT
        );
    }

    #[test]
    fn normalize_turn_context_message_limit_rejects_non_positive_values() {
        assert_eq!(normalize_turn_context_message_limit(Some(0), 123), 123);
        assert_eq!(normalize_turn_context_message_limit(Some(-50), 123), 123);
    }

    #[test]
    fn normalize_turn_context_message_limit_clamps_large_values() {
        assert_eq!(
            normalize_turn_context_message_limit(Some(i32::MAX), 123),
            MAX_TURN_CONTEXT_MESSAGE_LIMIT
        );
    }

    #[test]
    fn grpc_mcp_adapter_preserves_neutral_catalog_descriptors() {
        for (acts_as, wire_value) in [
            (everruns_core::McpServerActsAs::None, "none"),
            (everruns_core::McpServerActsAs::Service, "service"),
            (everruns_core::McpServerActsAs::User, "user"),
        ] {
            let resolved = crate::domains::mcp_servers::McpServerResolved {
                id: uuid::Uuid::new_v4(),
                name: "linear".to_string(),
                url: "https://mcp.linear.app/mcp".to_string(),
                auth_mode: everruns_core::McpServerAuthMode::None,
                protocol_mode: everruns_core::McpProtocolMode::Auto,
                oauth_provider_id: None,
                acts_as,
                api_key: None,
                headers: std::collections::HashMap::new(),
            };

            let proto = resolved_mcp_server_to_proto(resolved, std::collections::HashMap::new());

            assert_eq!(proto.acts_as, wire_value);
            assert_eq!(proto.auth_mode, "none");
            assert!(proto.oauth_provider_id.is_none());
            assert!(proto.api_key.is_none());
        }
    }
    #[test]
    fn ambiguous_bindings_do_not_rewrite_a_different_proto_tool() {
        let schema = serde_json::json!({"type":"object","properties":{"key":{"type":"string"}},"required":["key"]});
        let mut definitions = vec![super::McpToolDef {
            name: "mcp_docs___search".into(),
            description: "Search".into(),
            parameters: Some(everruns_internal_protocol::json_to_proto_struct(&schema)),
            ..Default::default()
        }];
        let before = definitions.clone();
        let mut binding = everruns_core::McpSecretBindingMetadata {
            server_name: "docs_".into(),
            tool_name: "search".into(),
            parameter_name: "key".into(),
            configured: true,
            setup_url: "/setup".into(),
        };
        super::apply_proto_secret_binding_schemas(&mut definitions, &[binding.clone()]);
        assert_eq!(definitions, before);
        binding.server_name = "docs".into();
        binding.tool_name = "_search".into();
        super::apply_proto_secret_binding_schemas(&mut definitions, &[binding]);
        assert_eq!(
            everruns_internal_protocol::proto_struct_to_json(
                definitions[0].parameters.as_ref().unwrap()
            ),
            serde_json::json!({"type":"object","properties":{},"required":[]})
        );
    }
}
