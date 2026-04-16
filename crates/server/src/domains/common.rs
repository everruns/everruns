// Domain command infrastructure.
//
// The Command trait, CommandError, CommandContext (Ctx), and inventory-based
// dispatch. See specs/domains.md for the full pattern spec.

use crate::storage::StorageBackend;
use axum::Json;
use axum::http::StatusCode;
use everruns_core::{Caller, Policy, PolicyError};
use serde::{Serialize, de::DeserializeOwned};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::api::common::ErrorResponse;

// ============================================================================
// CommandError — protocol-agnostic, adapters map to HTTP status / MCP string
// ============================================================================

#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    #[error("{0}")]
    BadRequest(String),
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
        }
    }
}

// ============================================================================
// Command trait
// ============================================================================

pub trait Command: DeserializeOwned + Send + 'static {
    type Output: Serialize + Send;

    /// Static metadata — drives MCP catalog generation.
    fn meta() -> CommandMeta;

    /// Policy to check before execution. None = public/no auth.
    fn policy() -> Option<&'static Policy> {
        None
    }

    /// Execute the command. Validation + business logic + persistence.
    fn execute(self, ctx: &Ctx) -> impl Future<Output = Result<Self::Output, CommandError>> + Send;
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
    pub dispatch: DispatchFn,
}

inventory::collect!(CommandDescriptor);

impl CommandDescriptor {
    /// Create a descriptor for a Command impl. Use with `inventory::submit!`.
    pub const fn of<C: Command>() -> Self {
        Self {
            meta: C::meta,
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
