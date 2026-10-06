// Generic command dispatch over HTTP.
//
// Routes:
//   GET  /v1/commands          the command contract this caller can use
//   POST /v1/commands/{name}   run one command
//
// Decision: one endpoint instead of one REST route per operation for clients
// that speak the command contract (`everruns-cli`). The grammar a person types
// and the grammar an agent types are already one artifact
// (`crates/cli-contract`); routing both through the same dispatch makes the
// behavior one artifact too. The REST routes stay for clients that want
// resource URLs.
//
// Decision: it reuses the `/mcp` state and the scripting pipeline
// (`catalog::dispatch_named`), so exposure, parameter normalization, policy
// (`Command::run`) and output decoration are identical to MCP `execute` and the
// Platform capability. Only the error mapping differs: Problem Details here.
//
// Decision: request and response are envelopes, not the bare params and
// output. Two things have to be able to change without breaking a client that
// already shipped: the contract as a whole (`api_version`, an exact match) and
// one command's shape (`schema_hash`). A stale hash is a warning, not an
// error, because the usual cause is an older CLI calling a command whose
// optional fields grew; the params are still validated as usual. New request
// fields must be added explicitly; unknown ones are rejected so a typo in an
// envelope field never reads as accepted.
//
// Decision: a mutating command may carry an `Idempotency-Key` header. The
// first request with a key runs the command and stores its response; a retry
// with the same key and the same request gets that response back, marked
// `Idempotent-Replayed: true`, instead of running the command again. The same
// key with a different request is a 422, and a retry while the first request
// is still running is a 409. Only successes are stored: a failed command
// releases its key so the client can retry it. Keys are scoped to the org and
// principal, remembered for a day, and an in-flight claim is a lease, so a
// request that died mid-command does not pin its key until it expires. The
// stored response is encrypted when the server has an encryption key: a
// command such as `create_eval_run_share` returns a token exactly once and
// otherwise keeps only its hash.
// Read-only commands ignore the header; running them twice is harmless.
//
// Decision: the claim commits on its own, before the command runs, so a
// concurrent retry sees the request in flight. For a transactional command
// (`Command::transactional`) the stored response is written inside the
// command's transaction, so the change, its history entry and the response a
// retry replays commit together; a failure to store it fails the request and
// rolls the change back. The release after a failure runs once that rollback
// is done.
//
// Decision: the envelope's `reason` is the change reason the command records in
// entity history (`domains::change_history`). It sits beside `params`, not in
// them, because it describes the invocation rather than the change, and it is
// left out of the idempotency fingerprint so a retry that rewords it is still
// the same request.
//
// Not to be confused with `/v1/sessions/{id}/commands`, which lists a
// session's slash commands (`api/commands.rs`).

use std::collections::BTreeMap;

use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use everruns_cli_contract::ContractCommand;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::common::ErrorResponse;
use super::mcp_endpoint::{AppState, catalog, catalog_context, cli_tree};
use crate::auth::ResolvedOrg;
use crate::domains::common::{CommandDescriptor, CommandError};
use crate::storage::command_idempotency::{
    ClaimIdempotencyKey, IdempotencyClaim, IdempotencyKeyScope,
};

/// API version of the command contract. Matches the internal gRPC transport.
pub const COMMAND_API_VERSION: &str = "v1";

/// Request header naming a retry-safe command request.
pub const IDEMPOTENCY_KEY_HEADER: &str = "idempotency-key";
/// Response header set when the response is a stored one.
pub const IDEMPOTENT_REPLAYED_HEADER: &str = "idempotent-replayed";
/// Longest accepted idempotency key.
const MAX_IDEMPOTENCY_KEY_LEN: usize = 255;
/// How long a key is remembered.
const IDEMPOTENCY_KEY_TTL: chrono::Duration = chrono::Duration::hours(24);
/// How long a request owns its key before a retry may take it over.
const IDEMPOTENCY_LOCK_TTL: chrono::Duration = chrono::Duration::minutes(10);

/// Most metadata entries one request may carry, and the longest key or value.
const MAX_METADATA_ENTRIES: usize = 16;
const MAX_METADATA_LEN: usize = 256;

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/commands", get(list_commands))
        .route("/v1/commands/{name}", post(execute_command))
        .route("/v1/history", get(super::history::list_org_history))
        .route(
            "/v1/history/{entity_ref}",
            get(super::history::list_entity_history),
        )
        .route(
            "/v1/history/{entity_ref}/revisions/{revision}",
            get(super::history::show_entity_revision),
        )
        .route(
            "/v1/history/{entity_ref}/diff",
            get(super::history::diff_entity_revisions),
        )
        .route(
            "/v1/history/{entity_ref}/restore",
            post(super::history::restore_entity_revision),
        )
        .route(
            "/v1/context/{entity_ref}",
            get(super::manager_context::get_manager_context)
                .put(super::manager_context::set_manager_context)
                .delete(super::manager_context::clear_manager_context),
        )
        .route(
            "/v1/context/{entity_ref}/append",
            post(super::manager_context::append_manager_context),
        )
        .with_state(state)
}

/// The command contract available to the caller.
#[derive(Debug, Serialize, ToSchema)]
pub struct CommandCatalog {
    /// Contract version; a client built for another version should not guess.
    #[schema(example = "v1")]
    pub api_version: &'static str,
    /// Every command the caller can dispatch.
    pub commands: Vec<CommandEntry>,
}

/// One command in the catalog: its command-line contract plus what a client
/// needs to call it safely.
#[derive(Debug, Serialize, ToSchema)]
pub struct CommandEntry {
    /// The `everruns-cli-contract` shape: wire name, noun path, verb,
    /// arguments, examples.
    #[serde(flatten)]
    #[schema(value_type = Object)]
    pub contract: &'static ContractCommand,
    /// Contract version this command is served under.
    #[schema(example = "v1")]
    pub api_version: &'static str,
    /// Fingerprint of the command's contract and parameter schema. A client
    /// echoes it on dispatch to learn when the command changed under it.
    #[schema(example = "3f1c9a0b5d2e4f6a")]
    pub schema_hash: String,
    /// Whether the command only reads.
    pub read_only: bool,
}

/// A command invocation.
#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CommandRequest {
    /// The command's params, keyed by field name as the contract lists it.
    #[serde(default)]
    #[schema(value_type = Object)]
    pub params: serde_json::Value,
    /// Contract version the client was built for. Defaults to the current one.
    #[serde(default)]
    #[schema(example = "v1")]
    pub api_version: Option<String>,
    /// `schema_hash` from the catalog the client was built against. A mismatch
    /// adds a warning to the response; it does not fail the call.
    #[serde(default)]
    pub schema_hash: Option<String>,
    /// Opaque client annotations (for example the client name and version),
    /// recorded on the request trace. Never interpreted by the command.
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
    /// Why the caller is making this change, recorded in the changed entity's
    /// history. 1 to 1000 characters; rejected if it looks like a credential.
    /// Read-only commands ignore it. Not part of the idempotency fingerprint:
    /// a retry may word it differently, and the first reason is kept.
    #[serde(default)]
    #[schema(example = "Make the support agent kid friendly, as requested in the product review")]
    pub reason: Option<String>,
    /// The changed entity's manager context revision the caller read
    /// (`get_manager_context`). A newer one refuses the change with
    /// `manager_context_changed`; omitting it while the entity has context
    /// adds a warning.
    #[serde(default)]
    pub context_revision: Option<i64>,
}

/// A command's result.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CommandResponse {
    /// Wire name of the command that ran.
    #[schema(example = "create_agent")]
    pub command: String,
    /// Contract version the command ran under.
    #[schema(example = "v1")]
    pub api_version: String,
    /// The command's current `schema_hash`.
    pub schema_hash: String,
    /// The command's output.
    #[schema(value_type = Object)]
    pub output: serde_json::Value,
    /// Non-fatal notices, such as a stale `schema_hash`.
    pub warnings: Vec<String>,
}

/// GET /v1/commands - List the command contract
#[utoipa::path(
    get,
    path = "/v1/commands",
    responses(
        (status = 200, description = "Commands the caller can dispatch", body = CommandCatalog),
        (status = 401, description = "Unauthorized", body = ErrorResponse),
    ),
    tag = "commands"
)]
pub async fn list_commands(org: ResolvedOrg) -> Json<CommandCatalog> {
    let commands = cli_tree::contracts_for(&org.feature_flags)
        .into_iter()
        .filter_map(|contract| {
            let desc = descriptor(&contract.wire_name)?;
            Some(CommandEntry {
                contract,
                api_version: COMMAND_API_VERSION,
                schema_hash: schema_hash(contract, desc),
                read_only: (desc.read_only)(),
            })
        })
        .collect();
    Json(CommandCatalog {
        api_version: COMMAND_API_VERSION,
        commands,
    })
}

/// POST /v1/commands/{name} - Run one command
#[utoipa::path(
    post,
    path = "/v1/commands/{name}",
    params(
        ("name" = String, Path, description = "Command wire name, e.g. create_agent"),
        ("Idempotency-Key" = Option<String>, Header, description = "Makes a mutating command safe to retry: a repeat with the same key and request returns the first response (with `Idempotent-Replayed: true`) instead of running again. Up to 255 printable ASCII characters, remembered for 24 hours per caller. Ignored by read-only commands."),
    ),
    request_body = CommandRequest,
    responses(
        (status = 200, description = "Command output", body = CommandResponse,
            headers(("Idempotent-Replayed" = String, description = "`true` when this is the stored response of an earlier request with the same Idempotency-Key"))),
        (status = 400, description = "Invalid request or params", body = ErrorResponse),
        (status = 403, description = "Forbidden", body = ErrorResponse),
        (status = 404, description = "Unknown command or resource", body = ErrorResponse),
        (status = 409, description = "A request with this Idempotency-Key is still running", body = ErrorResponse),
        (status = 422, description = "The Idempotency-Key was used for a different request", body = ErrorResponse),
    ),
    tag = "commands"
)]
pub async fn execute_command(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(name): Path<String>,
    headers: HeaderMap,
    body: Option<Json<CommandRequest>>,
) -> Result<Response, (StatusCode, Json<ErrorResponse>)> {
    let request = body.map(|Json(request)| request).unwrap_or_default();
    let key = idempotency_key(&headers)?;
    if let Some(version) = request.api_version.as_deref()
        && version != COMMAND_API_VERSION
    {
        return Err(CommandError::bad_request(format!(
            "Unsupported command api_version: {version} (server speaks {COMMAND_API_VERSION})"
        ))
        .into());
    }
    validate_metadata(&request.metadata)?;
    if !request.metadata.is_empty() {
        tracing::info!(command = %name, metadata = ?request.metadata, "command dispatch metadata");
    }

    let desc = descriptor(&name);
    let current_hash = match (
        desc,
        cli_tree::contracts()
            .iter()
            .find(|contract| contract.wire_name == name),
    ) {
        (Some(desc), Some(contract)) => schema_hash(contract, desc),
        _ => String::new(),
    };

    // Unknown and read-only commands run without a key: the first fails the
    // same way every time, the second is harmless to repeat.
    let Some(key) = key.filter(|_| desc.is_some_and(|desc| !(desc.read_only)())) else {
        let response = run(&org, &state, name, request, current_hash, None).await?;
        return Ok(Json(response).into_response());
    };
    let scope = IdempotencyKeyScope {
        org_id: org.org_id,
        principal_id: org.user_id.unwrap_or_default(),
        key,
    };
    let fingerprint = request_fingerprint(&name, &request.params);
    let now = chrono::Utc::now();
    let claim = state
        .db
        .claim_command_idempotency_key(&ClaimIdempotencyKey {
            scope: scope.clone(),
            command: name.clone(),
            fingerprint: fingerprint.clone(),
            locked_until: now + IDEMPOTENCY_LOCK_TTL,
            expires_at: now + IDEMPOTENCY_KEY_TTL,
        })
        .await
        .map_err(CommandError::internal)?;
    match claim {
        IdempotencyClaim::Existing(stored) if stored.fingerprint != fingerprint => {
            Err(CommandError::unprocessable(format!(
                "Idempotency-Key was already used for a different request ({}); \
                 use a new key for a new request",
                stored.command
            ))
            .with_code("idempotency_key_reused")
            .into())
        }
        IdempotencyClaim::Existing(stored) => match stored.response {
            Some(sealed) => {
                let mut response = Json(open_response(&state, &sealed)?).into_response();
                response
                    .headers_mut()
                    .insert(IDEMPOTENT_REPLAYED_HEADER, HeaderValue::from_static("true"));
                Ok(response)
            }
            None => Err(CommandError::conflict(
                "A request with this Idempotency-Key is still running; retry later",
            )
            .with_code("idempotency_key_in_progress")
            .into()),
        },
        IdempotencyClaim::Claimed => {
            let transactional = desc.is_some_and(|desc| (desc.transactional)());
            // Boxed so the command's deep future is not held inline twice.
            let attempt = Box::pin(async {
                let response = run(
                    &org,
                    &state,
                    name,
                    request,
                    current_hash,
                    Some(scope.key.clone()),
                )
                .await?;
                let stored = match seal_response(&state, &response) {
                    Ok(sealed) => {
                        state
                            .db
                            .complete_command_idempotency_key(&scope, &sealed)
                            .await
                    }
                    Err(err) => Err(err),
                };
                if let Err(err) = stored {
                    if transactional {
                        // Inside the command's transaction: the change and the
                        // response a retry replays commit together, or neither.
                        return Err(CommandError::internal(
                            err.context("failed to store the idempotent command response"),
                        )
                        .into());
                    }
                    // The command already committed: failing to remember it
                    // must not turn its success into an error.
                    tracing::warn!(error = %err, "failed to store idempotent command response");
                }
                Ok(response)
            });
            // The claim above committed on its own, so a concurrent retry sees
            // it in flight; the completion commits with the command.
            let result = if transactional {
                crate::storage::transaction::scope(&state.db, attempt, |err| {
                    CommandError::internal(err).into()
                })
                .await
            } else {
                attempt.await
            };
            match result {
                Ok(response) => Ok(Json(response).into_response()),
                Err(err) => {
                    // After the rollback: forget the in-flight claim so the
                    // client can retry.
                    if let Err(release) = state.db.release_command_idempotency_key(&scope).await {
                        tracing::warn!(error = %release, "failed to release idempotency key");
                    }
                    Err(err)
                }
            }
        }
    }
}

async fn run(
    org: &ResolvedOrg,
    state: &AppState,
    name: String,
    request: CommandRequest,
    current_hash: String,
    idempotency_key: Option<String>,
) -> Result<CommandResponse, (StatusCode, Json<ErrorResponse>)> {
    let mut context = catalog_context(org, state);
    let intent = crate::domains::change_history::ChangeIntent {
        idempotency_key: idempotency_key.clone(),
        context_revision: request.context_revision,
        ..crate::domains::change_history::ChangeIntent::on(
            crate::domains::change_history::ChangeSurface::Commands,
        )
    }
    .with_reason(request.reason);
    let notices = intent.notices.clone();
    context.domain_ctx = context.domain_ctx.with_change_intent(intent);
    let output = catalog::dispatch_named(&name, request.params, &context).await?;

    let mut warnings = notices.take();
    if let Some(sent) = request.schema_hash.as_deref()
        && sent != current_hash
    {
        warnings.push(format!(
            "schema_hash mismatch for {name}: the command changed since this client was built; \
             refresh its catalog"
        ));
    }
    Ok(CommandResponse {
        command: name,
        api_version: COMMAND_API_VERSION.to_string(),
        schema_hash: current_hash,
        output,
        warnings,
    })
}

/// A response as stored for replay: encrypted when the server has a key.
fn seal_response(state: &AppState, response: &CommandResponse) -> anyhow::Result<Vec<u8>> {
    let json = serde_json::to_vec(response)?;
    match &state.encryption {
        Some(encryption) => encryption.encrypt(&json),
        None => Ok(json),
    }
}

/// The stored response back. A row sealed before encryption was configured
/// (or after it was removed) still reads, as plain JSON.
fn open_response(state: &AppState, sealed: &[u8]) -> Result<serde_json::Value, CommandError> {
    let json = match &state.encryption {
        Some(encryption) => encryption
            .decrypt(sealed)
            .unwrap_or_else(|_| sealed.to_vec()),
        None => sealed.to_vec(),
    };
    serde_json::from_slice(&json).map_err(|err| {
        CommandError::internal(anyhow::anyhow!(
            "stored idempotent response is unreadable: {err}"
        ))
    })
}

/// The `Idempotency-Key` header, if the request sent one.
fn idempotency_key(headers: &HeaderMap) -> Result<Option<String>, CommandError> {
    let Some(value) = headers.get(IDEMPOTENCY_KEY_HEADER) else {
        return Ok(None);
    };
    let key = value.to_str().unwrap_or_default();
    if key.is_empty()
        || key.len() > MAX_IDEMPOTENCY_KEY_LEN
        || !key.bytes().all(|byte| byte.is_ascii_graphic())
    {
        return Err(CommandError::bad_request(format!(
            "Idempotency-Key must be 1 to {MAX_IDEMPOTENCY_KEY_LEN} printable ASCII characters"
        )));
    }
    Ok(Some(key.to_string()))
}

/// What makes two requests the same request: the command and its params,
/// with object keys sorted so field order does not matter. The envelope's
/// `schema_hash` and `metadata` describe the client, not the request.
pub fn request_fingerprint(name: &str, params: &serde_json::Value) -> String {
    use sha2::{Digest, Sha256};

    fn canonical(value: &serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::Object(map) => {
                let sorted: BTreeMap<&String, serde_json::Value> = map
                    .iter()
                    .map(|(key, value)| (key, canonical(value)))
                    .collect();
                serde_json::to_value(sorted).unwrap_or_default()
            }
            serde_json::Value::Array(items) => {
                serde_json::Value::Array(items.iter().map(canonical).collect())
            }
            other => other.clone(),
        }
    }

    let mut hasher = Sha256::new();
    hasher.update(name.as_bytes());
    hasher.update([0]);
    hasher.update(canonical(params).to_string().as_bytes());
    hex::encode(hasher.finalize())
}

fn validate_metadata(metadata: &BTreeMap<String, String>) -> Result<(), CommandError> {
    if metadata.len() > MAX_METADATA_ENTRIES
        || metadata
            .iter()
            .any(|(key, value)| key.len() > MAX_METADATA_LEN || value.len() > MAX_METADATA_LEN)
    {
        return Err(CommandError::bad_request(format!(
            "metadata allows at most {MAX_METADATA_ENTRIES} entries of {MAX_METADATA_LEN} bytes"
        )));
    }
    Ok(())
}

fn descriptor(name: &str) -> Option<&'static CommandDescriptor> {
    inventory::iter::<CommandDescriptor>
        .into_iter()
        .find(|desc| (desc.meta)().name == name)
}

/// Fingerprint of what a client depends on: the command-line contract (names,
/// flags, kinds, positionals) and the parameter schema the server validates.
pub(crate) fn schema_hash(contract: &ContractCommand, desc: &CommandDescriptor) -> String {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(serde_json::to_vec(contract).unwrap_or_default());
    hasher.update([0]);
    hasher.update(serde_json::to_vec(&(desc.param_schema)()).unwrap_or_default());
    hex::encode(&hasher.finalize()[..8])
}
