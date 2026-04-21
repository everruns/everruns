// Domain command infrastructure.
//
// The Command trait, CommandError, CommandContext (Ctx), and inventory-based
// dispatch. See specs/domains.md for the full pattern spec.

use crate::storage::StorageBackend;
use axum::Json;
use axum::http::StatusCode;
use everruns_core::{Caller, Policy, PolicyError};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use utoipa::ToSchema;

use crate::api::common::ErrorResponse;

// ============================================================================
// CommandError — protocol-agnostic, adapters map to HTTP status / MCP string
// ============================================================================

#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    #[error("{0}")]
    BadRequest(String),
    #[error("{0}")]
    Unprocessable(String),
    #[error("{0}")]
    Forbidden(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    Internal(#[from] anyhow::Error),
}

impl CommandError {
    pub fn status(&self) -> StatusCode {
        match self {
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Unprocessable(_) => StatusCode::UNPROCESSABLE_ENTITY,
            Self::Forbidden(_) => StatusCode::FORBIDDEN,
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    pub fn not_found(resource: &str) -> Self {
        Self::NotFound(format!("{resource} not found"))
    }

    pub fn bad_request(msg: impl Into<String>) -> Self {
        Self::BadRequest(msg.into())
    }

    pub fn unprocessable(msg: impl Into<String>) -> Self {
        Self::Unprocessable(msg.into())
    }

    pub fn conflict(msg: impl Into<String>) -> Self {
        Self::Conflict(msg.into())
    }
}

/// Convert anyhow::Error to CommandError, classifying known error patterns.
pub fn classify_anyhow(e: anyhow::Error) -> CommandError {
    // PolicyError → Forbidden
    if let Some(pe) = e.downcast_ref::<PolicyError>() {
        return CommandError::Forbidden(pe.message.clone());
    }

    // BadRequestError → BadRequest
    if let Some(br) = e.downcast_ref::<crate::errors::BadRequestError>() {
        return CommandError::BadRequest(br.message().to_string());
    }

    // ResourceNotFoundError → NotFound
    if let Some(nf) = e.downcast_ref::<crate::errors::ResourceNotFoundError>() {
        return CommandError::NotFound(format!("{} not found", nf.resource()));
    }

    let msg = e.to_string();
    let lowered = msg.to_ascii_lowercase();

    if lowered.contains("duplicate key") || lowered.contains("already exists") {
        return CommandError::Conflict(msg);
    }

    let is_bad_request = [
        "cannot be assigned",
        "cannot be edited",
        "must be archived before deletion",
        "cannot delete built-in",
        "cannot modify built-in",
        "cannot publish archived",
        "cannot unpublish archived",
        "cannot update archived",
        "cannot archive built-in",
        "invalid mcp capability reference",
        "invalid skill capability reference",
        "unsupported locale",
        "unsupported timezone",
    ]
    .iter()
    .any(|pattern| lowered.contains(pattern));

    if is_bad_request {
        return CommandError::BadRequest(msg);
    }

    CommandError::Internal(e)
}

// ============================================================================
// HTTP error conversion
// ============================================================================

impl From<CommandError> for (StatusCode, Json<ErrorResponse>) {
    fn from(e: CommandError) -> Self {
        let status = e.status();
        match &e {
            CommandError::Internal(inner) => {
                tracing::error!("Command error: {inner}");
                (status, Json(ErrorResponse::new("Internal server error")))
            }
            _ => (status, Json(ErrorResponse::new(e.to_string()))),
        }
    }
}

// ============================================================================
// CommandMeta — static metadata for catalog generation
// ============================================================================

#[derive(Debug, Clone)]
pub struct CommandMeta {
    pub name: &'static str,
    pub category: &'static str,
    pub description: &'static str,
    pub method: &'static str,
    pub path: &'static str,
}

// ============================================================================
// Ctx — shared execution context
// ============================================================================

#[derive(Clone)]
pub struct Ctx {
    pub caller: Caller,
    pub db: Arc<StorageBackend>,
    pub capability_service: Arc<crate::services::CapabilityService>,
    pub encryption: Option<Arc<crate::storage::encryption::EncryptionService>>,
    pub session_service: Option<Arc<crate::services::SessionService>>,
    pub message_service: Option<Arc<crate::services::MessageService>>,
    pub event_service: Option<Arc<crate::services::EventService>>,
    pub session_file_service: Option<Arc<crate::services::SessionFileService>>,
    pub session_resource_service: Option<Arc<crate::services::SessionResourceService>>,
    pub session_schedule_service: Option<Arc<crate::services::SessionScheduleService>>,
    pub notification_service: Option<Arc<crate::services::NotificationService>>,
    pub llm_model_service: Option<Arc<crate::services::LlmModelService>>,
    pub llm_provider_service: Option<Arc<crate::services::LlmProviderService>>,
    pub model_sync_service: Option<Arc<crate::services::ModelSyncService>>,
    pub eval_service: Option<Arc<crate::services::EvalService>>,
    pub sqldb_store: Option<Arc<dyn everruns_core::session_sqldb::SessionSqlDbStore>>,
    pub workflow_store: Option<Arc<dyn everruns_durable::WorkflowEventStore + Send + Sync>>,
    pub runner: Option<Arc<dyn everruns_worker::AgentRunner>>,
    pub fallback_harness_name: Option<String>,
    pub chat_harness_name: Option<String>,
    pub chat_session_title: Option<String>,
}

impl Ctx {
    pub fn org_id(&self) -> i64 {
        self.caller.org_id
    }

    /// Construct a Ctx for an HTTP request.
    ///
    /// `encryption` may be `None` for domains that never need it (agents,
    /// harnesses, skills). Domains that encrypt secrets (apps, mcp_servers)
    /// must pass `Some`.
    pub fn new(
        caller: Caller,
        db: Arc<StorageBackend>,
        capability_service: Arc<crate::services::CapabilityService>,
        encryption: Option<Arc<crate::storage::encryption::EncryptionService>>,
    ) -> Self {
        Self {
            caller,
            db,
            capability_service,
            encryption,
            session_service: None,
            message_service: None,
            event_service: None,
            session_file_service: None,
            session_resource_service: None,
            session_schedule_service: None,
            notification_service: None,
            llm_model_service: None,
            llm_provider_service: None,
            model_sync_service: None,
            eval_service: None,
            sqldb_store: None,
            workflow_store: None,
            runner: None,
            fallback_harness_name: None,
            chat_harness_name: None,
            chat_session_title: None,
        }
    }

    pub fn minimal(
        caller: Caller,
        db: Arc<StorageBackend>,
        encryption: Option<Arc<crate::storage::encryption::EncryptionService>>,
    ) -> Self {
        let capability_service =
            Arc::new(crate::services::CapabilityService::new(db.clone(), None));
        Self::new(caller, db, capability_service, encryption)
    }

    pub fn with_session_service(mut self, service: Arc<crate::services::SessionService>) -> Self {
        self.session_service = Some(service);
        self
    }

    pub fn with_message_service(mut self, service: Arc<crate::services::MessageService>) -> Self {
        self.message_service = Some(service);
        self
    }

    pub fn with_event_service(mut self, service: Arc<crate::services::EventService>) -> Self {
        self.event_service = Some(service);
        self
    }

    pub fn with_session_file_service(
        mut self,
        service: Arc<crate::services::SessionFileService>,
    ) -> Self {
        self.session_file_service = Some(service);
        self
    }

    pub fn with_session_resource_service(
        mut self,
        service: Arc<crate::services::SessionResourceService>,
    ) -> Self {
        self.session_resource_service = Some(service);
        self
    }

    pub fn with_session_schedule_service(
        mut self,
        service: Arc<crate::services::SessionScheduleService>,
    ) -> Self {
        self.session_schedule_service = Some(service);
        self
    }

    pub fn with_notification_service(
        mut self,
        service: Arc<crate::services::NotificationService>,
    ) -> Self {
        self.notification_service = Some(service);
        self
    }

    pub fn with_llm_model_service(
        mut self,
        service: Arc<crate::services::LlmModelService>,
    ) -> Self {
        self.llm_model_service = Some(service);
        self
    }

    pub fn with_llm_provider_service(
        mut self,
        service: Arc<crate::services::LlmProviderService>,
    ) -> Self {
        self.llm_provider_service = Some(service);
        self
    }

    pub fn with_model_sync_service(
        mut self,
        service: Arc<crate::services::ModelSyncService>,
    ) -> Self {
        self.model_sync_service = Some(service);
        self
    }

    pub fn with_eval_service(mut self, service: Arc<crate::services::EvalService>) -> Self {
        self.eval_service = Some(service);
        self
    }

    pub fn with_sqldb_store(
        mut self,
        store: Arc<dyn everruns_core::session_sqldb::SessionSqlDbStore>,
    ) -> Self {
        self.sqldb_store = Some(store);
        self
    }

    pub fn with_workflow_store(
        mut self,
        store: Arc<dyn everruns_durable::WorkflowEventStore + Send + Sync>,
    ) -> Self {
        self.workflow_store = Some(store);
        self
    }

    pub fn with_runner(mut self, runner: Arc<dyn everruns_worker::AgentRunner>) -> Self {
        self.runner = Some(runner);
        self
    }

    pub fn with_fallback_harness_name(mut self, name: Option<String>) -> Self {
        self.fallback_harness_name = name;
        self
    }

    pub fn with_chat_harness_name(mut self, name: Option<String>) -> Self {
        self.chat_harness_name = name;
        self
    }

    pub fn with_chat_session_title(mut self, title: Option<String>) -> Self {
        self.chat_session_title = title;
        self
    }
}

// ============================================================================
// Command trait
// ============================================================================

pub trait Command: DeserializeOwned + Send + 'static + CommandSchema {
    type Output: Serialize + Send;

    /// Static metadata — drives MCP catalog generation.
    fn meta() -> CommandMeta;

    /// Policy to check before execution. None = public/no auth.
    fn policy() -> Option<&'static Policy> {
        None
    }

    /// If set, allows MCP `execute` callers to pass the value for this field as
    /// a single positional argument (e.g. `get_agent <id>` instead of
    /// `get_agent --id <id>`). bashkit's flag parser hardcodes `expected --flag`
    /// for positional args, so the command string is pre-rewritten to insert
    /// `--<positional_arg>` before the value. See EVE-323.
    fn positional_arg() -> Option<&'static str> {
        None
    }

    /// JSON Schema for the command input surfaced in the MCP catalog.
    fn param_schema() -> Value {
        json_schema_for_command::<Self>()
    }

    /// Execute the command. Validation + business logic + persistence.
    fn execute(self, ctx: &Ctx) -> impl Future<Output = Result<Self::Output, CommandError>> + Send;
}

pub trait CommandSchema {
    fn param_schema() -> Value;
}

impl<T> CommandSchema for T
where
    T: ToSchema,
{
    fn param_schema() -> Value {
        json_schema_for::<T>()
    }
}

fn json_schema_for_command<T>() -> Value
where
    T: Command + CommandSchema,
{
    <T as CommandSchema>::param_schema()
}

pub fn delegated_param_schema<T>() -> Value
where
    T: ToSchema,
{
    json_schema_for::<T>()
}

fn json_schema_for<T>() -> Value
where
    T: ToSchema,
{
    let mut schema = match serde_json::to_value(T::schema()) {
        Ok(schema) => schema,
        Err(err) => {
            tracing::warn!(
                schema_type = std::any::type_name::<T>(),
                error = %err,
                "failed to serialize command schema; falling back to open object schema"
            );
            return open_object_schema();
        }
    };

    rewrite_schema_refs(&mut schema);

    let mut defs = serde_json::Map::new();
    let mut refs = Vec::new();
    T::schemas(&mut refs);
    for (name, ref_or_schema) in refs {
        let mut value = match serde_json::to_value(ref_or_schema) {
            Ok(value) => value,
            Err(err) => {
                tracing::warn!(
                    schema_type = std::any::type_name::<T>(),
                    schema_name = %name,
                    error = %err,
                    "failed to serialize referenced schema; using open object schema in $defs"
                );
                open_object_schema()
            }
        };
        rewrite_schema_refs(&mut value);
        defs.insert(name, value);
    }

    if !defs.is_empty()
        && let Some(obj) = schema.as_object_mut()
    {
        obj.insert("$defs".to_string(), Value::Object(defs));
    }

    schema
}

fn open_object_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": true,
    })
}

fn rewrite_schema_refs(value: &mut Value) {
    match value {
        Value::Object(map) => {
            if let Some(Value::String(reference)) = map.get_mut("$ref")
                && let Some(name) = reference.strip_prefix("#/components/schemas/")
            {
                *reference = format!("#/$defs/{name}");
            }

            for child in map.values_mut() {
                rewrite_schema_refs(child);
            }
        }
        Value::Array(items) => {
            for item in items {
                rewrite_schema_refs(item);
            }
        }
        _ => {}
    }
}

// ============================================================================
// Paginated — generic list response
// ============================================================================

#[derive(Debug, Clone, Serialize)]
pub struct Paginated<T: Serialize> {
    pub data: Vec<T>,
    pub total: u32,
    pub offset: u32,
    pub limit: u32,
}

// ============================================================================
// CommandDescriptor — type-erased entry for inventory
// ============================================================================

type DispatchFn =
    for<'a> fn(
        serde_json::Value,
        &'a Ctx,
    ) -> Pin<Box<dyn Future<Output = Result<String, CommandError>> + Send + 'a>>;

pub struct CommandDescriptor {
    pub meta: fn() -> CommandMeta,
    pub positional_arg: fn() -> Option<&'static str>,
    pub param_schema: fn() -> Value,
    pub dispatch: DispatchFn,
}

inventory::collect!(CommandDescriptor);

impl CommandDescriptor {
    /// Create a descriptor for a Command impl. Use with `inventory::submit!`.
    pub const fn of<C: Command>() -> Self {
        Self {
            meta: C::meta,
            positional_arg: C::positional_arg,
            param_schema: <C as Command>::param_schema,
            dispatch: dispatch_for::<C>,
        }
    }
}

fn dispatch_for<C: Command>(
    params: serde_json::Value,
    ctx: &Ctx,
) -> Pin<Box<dyn Future<Output = Result<String, CommandError>> + Send + '_>> {
    Box::pin(async move {
        // Policy check
        if let Some(policy) = C::policy() {
            policy
                .evaluate(&ctx.caller)
                .map_err(|e| CommandError::Forbidden(e.message))?;
        }

        let cmd: C =
            serde_json::from_value(params).map_err(|e| CommandError::BadRequest(e.to_string()))?;

        let result = cmd.execute(ctx).await?;

        serde_json::to_string(&result).map_err(|e| CommandError::Internal(e.into()))
    })
}

// ============================================================================
// Dispatch table — built once, used by MCP endpoint
// ============================================================================

use std::collections::HashMap;
use std::sync::LazyLock;

static DISPATCH_TABLE: LazyLock<HashMap<&'static str, &'static CommandDescriptor>> =
    LazyLock::new(|| {
        inventory::iter::<CommandDescriptor>
            .into_iter()
            .map(|desc| ((desc.meta)().name, desc))
            .collect()
    });

/// Dispatch a command by name from JSON params. Used by MCP endpoint.
pub async fn dispatch(
    name: &str,
    params: serde_json::Value,
    ctx: &Ctx,
) -> Result<String, CommandError> {
    let desc = DISPATCH_TABLE
        .get(name)
        .ok_or_else(|| CommandError::NotFound(format!("Unknown command: {name}")))?;
    (desc.dispatch)(params, ctx).await
}

/// Build catalog entries from all registered commands.
pub fn catalog_entries() -> Vec<CommandMeta> {
    inventory::iter::<CommandDescriptor>
        .into_iter()
        .map(|desc| (desc.meta)())
        .collect()
}

// ============================================================================
// Validation helpers shared across domains
// ============================================================================

/// Validate an addressable name (agent, harness, etc.)
pub fn validate_name(entity: &str, name: &str) -> Result<(), CommandError> {
    everruns_core::validate_addressable_name(name)
        .map_err(|msg| CommandError::bad_request(format!("{entity} {msg}")))
}

/// Clamp pagination params to safe defaults.
pub fn pagination(offset: Option<u32>, limit: Option<u32>) -> crate::api::common::Pagination {
    let offset = offset.unwrap_or(0);
    let limit = limit.unwrap_or(20).min(100);
    crate::api::common::Pagination::new(offset, limit)
}

// ============================================================================
// Lenient deserializers — accept both typed JSON values and stringly-typed
// values from the MCP CLI bridge (bashkit catalog).
// ============================================================================
//
// Context (EVE-324): inventory-registered commands expose an open JSON schema
// (`additionalProperties: true`) to bashkit because the request types own
// their own serde shape. Without per-flag type hints, bashkit's flag parser
// defaults every value to a string — so `list_capabilities --limit 5` arrives
// at dispatch as `{"limit": "5"}`. Serde then rejects that string while
// trying to populate `Option<u32>`, the dispatcher wraps the error as
// `CommandError::BadRequest`, and the bashkit adapter sanitizes it into the
// opaque "<cmd>: callback failed" the caller sees.
//
// The helpers below sit between the serde field and the raw JSON: they accept
// the native typed shape (for programmatic callers) and also coerce the
// string forms bashkit produces. They are intentionally scoped to the input
// fields that are known to arrive from the flag parser — primarily pagination
// (`offset`, `limit`) and boolean toggles (`include_archived`).
/// Accept `Option<u32>` as `null`, an integer, or a numeric string.
pub fn deserialize_opt_u32_lenient<'de, D>(d: D) -> Result<Option<u32>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Either {
        Num(u32),
        Str(String),
    }

    match Option::<Either>::deserialize(d)? {
        None => Ok(None),
        Some(Either::Num(n)) => Ok(Some(n)),
        Some(Either::Str(s)) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                return Ok(None);
            }
            trimmed
                .parse::<u32>()
                .map(Some)
                .map_err(serde::de::Error::custom)
        }
    }
}

/// Accept `bool`, `"true"`/`"false"`/`"1"`/`"0"`/`"yes"`/`"no"` (case-insensitive),
/// or integer `0`/`1`. Missing keys fall through to serde's default handling.
pub fn deserialize_bool_lenient<'de, D>(d: D) -> Result<bool, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Either {
        Bool(bool),
        Num(i64),
        Str(String),
    }

    match Either::deserialize(d)? {
        Either::Bool(b) => Ok(b),
        Either::Num(0) => Ok(false),
        Either::Num(1) => Ok(true),
        Either::Num(n) => Err(serde::de::Error::custom(format!(
            "cannot coerce integer {n} to bool (expected 0 or 1)"
        ))),
        Either::Str(s) => match s.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" | "y" | "on" => Ok(true),
            "false" | "0" | "no" | "n" | "off" => Ok(false),
            other => Err(serde::de::Error::custom(format!(
                "cannot coerce string {other:?} to bool"
            ))),
        },
    }
}

#[cfg(test)]
mod error_tests {
    use super::*;

    #[test]
    fn classify_anyhow_maps_bad_request_error() {
        let err = classify_anyhow(crate::errors::BadRequestError::new("bad input").into());
        assert!(matches!(err, CommandError::BadRequest(msg) if msg == "bad input"));
    }

    #[test]
    fn classify_anyhow_maps_not_found_error() {
        let err = classify_anyhow(crate::errors::ResourceNotFoundError::new("Thing").into());
        assert!(matches!(err, CommandError::NotFound(msg) if msg == "Thing not found"));
    }
}

#[cfg(test)]
mod lenient_tests {
    use super::*;
    use serde::Deserialize;
    use serde_json::json;

    #[derive(Debug, Deserialize, PartialEq)]
    struct PageInput {
        #[serde(default, deserialize_with = "deserialize_opt_u32_lenient")]
        offset: Option<u32>,
        #[serde(default, deserialize_with = "deserialize_opt_u32_lenient")]
        limit: Option<u32>,
    }

    #[derive(Debug, Deserialize, PartialEq)]
    struct FlagInput {
        #[serde(default, deserialize_with = "deserialize_bool_lenient")]
        include_archived: bool,
    }

    #[test]
    fn opt_u32_lenient_accepts_integer() {
        let v: PageInput = serde_json::from_value(json!({"offset": 0, "limit": 5})).unwrap();
        assert_eq!(
            v,
            PageInput {
                offset: Some(0),
                limit: Some(5)
            }
        );
    }

    #[test]
    fn opt_u32_lenient_accepts_numeric_string() {
        let v: PageInput = serde_json::from_value(json!({"offset": "10", "limit": "20"})).unwrap();
        assert_eq!(
            v,
            PageInput {
                offset: Some(10),
                limit: Some(20)
            }
        );
    }

    #[test]
    fn opt_u32_lenient_accepts_missing_and_null() {
        let missing: PageInput = serde_json::from_value(json!({})).unwrap();
        assert_eq!(
            missing,
            PageInput {
                offset: None,
                limit: None
            }
        );
        let nulled: PageInput =
            serde_json::from_value(json!({"offset": null, "limit": null})).unwrap();
        assert_eq!(
            nulled,
            PageInput {
                offset: None,
                limit: None
            }
        );
    }

    #[test]
    fn opt_u32_lenient_rejects_non_numeric_string() {
        let err = serde_json::from_value::<PageInput>(json!({"limit": "abc"})).unwrap_err();
        assert!(err.to_string().contains("invalid digit"), "got: {err}");
    }

    #[test]
    fn bool_lenient_accepts_variants() {
        for (input, expected) in [
            (json!(true), true),
            (json!(false), false),
            (json!("true"), true),
            (json!("FALSE"), false),
            (json!("1"), true),
            (json!("0"), false),
            (json!("yes"), true),
            (json!("no"), false),
            (json!(1), true),
            (json!(0), false),
        ] {
            let v: FlagInput = serde_json::from_value(json!({"include_archived": input})).unwrap();
            assert_eq!(v.include_archived, expected, "input: {input}");
        }
    }

    #[test]
    fn bool_lenient_rejects_garbage() {
        let err =
            serde_json::from_value::<FlagInput>(json!({"include_archived": "maybe"})).unwrap_err();
        assert!(err.to_string().contains("coerce string"), "got: {err}");
    }
}
