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
// Not to be confused with `/v1/sessions/{id}/commands`, which lists a
// session's slash commands (`api/commands.rs`).

use std::collections::BTreeMap;

use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, post},
};
use everruns_cli_contract::ContractCommand;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::common::{ApiResult, ErrorResponse};
use super::mcp_endpoint::{AppState, catalog, catalog_context, cli_tree};
use crate::auth::ResolvedOrg;
use crate::domains::common::{CommandDescriptor, CommandError};

/// API version of the command contract. Matches the internal gRPC transport.
pub const COMMAND_API_VERSION: &str = "v1";

/// Most metadata entries one request may carry, and the longest key or value.
const MAX_METADATA_ENTRIES: usize = 16;
const MAX_METADATA_LEN: usize = 256;

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/commands", get(list_commands))
        .route("/v1/commands/{name}", post(execute_command))
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
}

/// A command's result.
#[derive(Debug, Serialize, ToSchema)]
pub struct CommandResponse {
    /// Wire name of the command that ran.
    #[schema(example = "create_agent")]
    pub command: String,
    /// Contract version the command ran under.
    #[schema(example = "v1")]
    pub api_version: &'static str,
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
    params(("name" = String, Path, description = "Command wire name, e.g. create_agent")),
    request_body = CommandRequest,
    responses(
        (status = 200, description = "Command output", body = CommandResponse),
        (status = 400, description = "Invalid request or params", body = ErrorResponse),
        (status = 403, description = "Forbidden", body = ErrorResponse),
        (status = 404, description = "Unknown command or resource", body = ErrorResponse),
    ),
    tag = "commands"
)]
pub async fn execute_command(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(name): Path<String>,
    body: Option<Json<CommandRequest>>,
) -> ApiResult<CommandResponse> {
    let request = body.map(|Json(request)| request).unwrap_or_default();
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

    let current_hash = match (
        descriptor(&name),
        cli_tree::contracts()
            .iter()
            .find(|contract| contract.wire_name == name),
    ) {
        (Some(desc), Some(contract)) => schema_hash(contract, desc),
        _ => String::new(),
    };
    let output =
        catalog::dispatch_named(&name, request.params, &catalog_context(&org, &state)).await?;

    let mut warnings = Vec::new();
    if let Some(sent) = request.schema_hash.as_deref()
        && sent != current_hash
    {
        warnings.push(format!(
            "schema_hash mismatch for {name}: the command changed since this client was built; \
             refresh its catalog"
        ));
    }
    Ok(Json(CommandResponse {
        command: name,
        api_version: COMMAND_API_VERSION,
        schema_hash: current_hash,
        output,
        warnings,
    }))
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
