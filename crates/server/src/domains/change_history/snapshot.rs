// Snapshots, revisions and restore.
//
// Decision: a snapshot is the entity as its own read command returns it (the
// kind's `lookup`), or, for kinds read through a parent, the command's output
// when that output is the entity. That is the public shape every surface
// already shows, so it holds no plaintext secret, and restore can feed it
// straight back into the kind's update command. Volatile fields (timestamps,
// links, hypermedia) are dropped so a no-op update hashes the same.
//
// Secrets never enter a snapshot. Each secret column of a kind becomes a
// marker under `$secrets`: whether it is set, and a keyed fingerprint of its
// value (`EncryptionService::fingerprint`), so history can say "the key
// changed" and nothing more. Restore keeps every secret's current value and
// warns when its fingerprint differs from the restored revision's.
//
// Design: knowledge/execution/change-reasons-and-manager-context.md
// ("Revisions, restore and secrets").

use serde_json::{Map, Value};

use super::{ChangeAction, EntityKind};
use crate::domains::common::{CommandError, Ctx, dispatch};

/// Update params restore never sends, even when a snapshot shows them: they
/// hold secrets, or config whose secrets reads redact, so sending the read
/// value back would drop or overwrite a secret.
const NEVER_RESTORED: &[&str] = &[
    "api_key",
    "private_key",
    "token",
    "auth",
    "credential",
    "channel_config",
];

/// Where a snapshot keeps its secret markers. Not a field of any entity, so
/// restore never sends it to an update command.
pub const SECRETS_KEY: &str = "$secrets";

/// Fields that change without anyone changing the entity.
const VOLATILE: &[&str] = &[
    "created_at",
    "updated_at",
    "last_used_at",
    "last_synced_at",
    "models_reviewed_at",
    "last_triggered_at",
    "next_trigger_at",
    "claimed_at",
    "claimed_by",
    "ui_link",
    "self_url",
    "view_url",
    "allowed_actions",
    "links",
    // Derived from other entities, not part of this one's configuration.
    "session_count",
    "app_count",
    "channels",
    "exposed",
    "exposures_suspended",
    "effective_harness",
];

/// Whether changes to `kind` carry snapshots. Sessions change by
/// conversation, not by configuration, so theirs are not kept.
pub fn has_snapshots(kind: EntityKind) -> bool {
    kind != EntityKind::Session
}

/// The UUID inside a public ref (`provider_<hex>`) or a bare UUID ref.
pub fn ref_uuid(entity_ref: &str) -> Option<uuid::Uuid> {
    uuid::Uuid::parse_str(entity_ref).ok().or_else(|| {
        let (_, hex) = entity_ref.rsplit_once('_')?;
        uuid::Uuid::parse_str(hex).ok()
    })
}

/// The entity's snapshot after a change and its hash, or `None` when the kind
/// keeps none, the change deleted it, or its state cannot be read.
pub(super) async fn capture(
    ctx: &Ctx,
    kind: EntityKind,
    action: ChangeAction,
    entity_ref: &str,
    output: &Value,
) -> Option<(Value, String)> {
    if !has_snapshots(kind) || action == ChangeAction::Deleted {
        return None;
    }
    let state = match read(ctx, kind, entity_ref).await {
        Some(state) => state,
        None => output_entity(output, entity_ref)?,
    };
    let mut snapshot = normalize(state);
    if kind == EntityKind::Skill
        && let Some(fields) = snapshot.as_object_mut()
    {
        fields.retain(|key, _| matches!(key.as_str(), "id" | "status" | "skill_md"));
    }
    if kind == EntityKind::Provider {
        snapshot["request_options"] = provider_options(&snapshot["request_options"]);
        snapshot["trace"] = provider_trace(&snapshot["trace"]);
    }
    if kind == EntityKind::AgentChannel {
        let channel_type = snapshot["channel_type"].as_str().unwrap_or("").to_string();
        snapshot["channel_config"] = crate::domains::agent_channels::history::public_config(
            &channel_type,
            &snapshot["channel_config"],
        );
    }
    if kind == EntityKind::OrganizationNetwork {
        // Grants, open egress and deployment mode are separate authorities.
        if let Some(fields) = snapshot.as_object_mut() {
            fields.retain(|key, _| key == "patterns");
        }
    }
    if kind == EntityKind::Knowledge {
        // Content has its own immutable versions. Configuration snapshots must
        // not change when workers advance jobs, file counts or source cursors.
        if let Some(object) = snapshot.as_object_mut() {
            for field in [
                "revision",
                "published_revision",
                "serving_config",
                "file_count",
                "indexed_count",
                "jobs",
            ] {
                object.remove(field);
            }
            if let Some(sources) = object.get_mut("sources").and_then(Value::as_array_mut) {
                for source in sources {
                    if let Some(fields) = source.as_object_mut() {
                        fields.retain(|key, _| {
                            matches!(
                                key.as_str(),
                                "id" | "kind" | "root_path" | "config" | "connection_owner"
                            )
                        });
                    }
                }
            }
        }
    }
    let secrets = secret_markers(ctx, kind, entity_ref, &snapshot).await;
    if !secrets.is_empty()
        && let Some(object) = snapshot.as_object_mut()
    {
        object.insert(SECRETS_KEY.to_string(), Value::Object(secrets));
    }
    let hash = hash(&snapshot);
    Some((snapshot, hash))
}

fn provider_options(value: &Value) -> Value {
    serde_json::json!({"cache_diagnostics": value.get("cache_diagnostics").and_then(Value::as_bool).unwrap_or(false)})
}
fn provider_trace(value: &Value) -> Value {
    serde_json::from_value::<everruns_contracts::provider::ProviderTraceConfig>(value.clone())
        .ok()
        .and_then(|value| serde_json::to_value(value).ok())
        .unwrap_or_else(|| serde_json::json!({"enabled":false}))
}

/// The entity as its read command returns it now.
pub(super) async fn read(ctx: &Ctx, kind: EntityKind, entity_ref: &str) -> Option<Value> {
    if kind == EntityKind::VirtualUser
        && crate::domains::virtual_users::self_profile::owns(ctx, entity_ref).await
    {
        let text = dispatch(
            "get_own_virtual_user_profile",
            serde_json::json!({"id":entity_ref}),
            ctx,
        )
        .await
        .ok()?;
        return serde_json::from_str(&text).ok();
    }
    let (command, param) = match kind.lookup() {
        Some(lookup) => lookup,
        None => return read_child(ctx, kind, entity_ref).await,
    };
    let mut params = Map::new();
    params.insert(param.to_string(), entity_ref.into());
    let text = dispatch(command, Value::Object(params), ctx).await.ok()?;
    let mut state: Value = serde_json::from_str(&text).ok()?;
    if kind == EntityKind::Skill {
        let content = dispatch(
            "get_skill_content",
            serde_json::json!({"id":entity_ref}),
            ctx,
        )
        .await
        .ok()?;
        let content: Value = serde_json::from_str(&content).ok()?;
        state["skill_md"] = content.get("skill_md")?.clone();
    }
    Some(state)
}

// Resolve only the immutable locator inside the active tenant, then use the
// normal read command for its full permission and visibility checks.
async fn read_child(ctx: &Ctx, kind: EntityKind, entity_ref: &str) -> Option<Value> {
    let id = ref_uuid(entity_ref)?;
    let (sql, command, parent, child) = match kind {
        EntityKind::AgentChannel => (
            "SELECT a.public_id FROM agent_channels c JOIN agents a ON a.id=c.agent_id WHERE a.org_id=$1 AND c.public_id='appchan_' || replace($2::uuid::text,'-','')",
            "get_agent_channel",
            "agent_id",
            "channel_id",
        ),
        EntityKind::AgentTrigger => (
            "SELECT a.public_id FROM agent_triggers c JOIN agents a ON a.id=c.agent_id WHERE a.org_id=$1 AND c.id=$2",
            "get_agent_trigger",
            "agent_id",
            "trigger_id",
        ),
        EntityKind::AgentScript => (
            "SELECT a.public_id FROM agent_scripts c JOIN agents a ON a.id=c.agent_id WHERE a.org_id=$1 AND c.id=$2",
            "get_agent_script",
            "agent_id",
            "script_id",
        ),
        EntityKind::KnowledgeEntry => (
            "SELECT a.public_id FROM knowledge_entries c JOIN knowledge_bases a ON a.id=c.kb_id WHERE a.org_id=$1 AND c.public_id='kbe_' || replace($2::uuid::text,'-','')",
            "get_knowledge_entry",
            "kb_id",
            "entry_id",
        ),
        EntityKind::EvalCase => (
            "SELECT a.public_id FROM eval_cases c JOIN evals a ON a.id=c.eval_id WHERE a.org_id=$1 AND c.public_id='evalcase_' || replace($2::uuid::text,'-','')",
            "get_eval_case",
            "eval_id",
            "case_id",
        ),
        _ => return None,
    };
    let locator: String = sqlx::query_scalar(sql)
        .bind(ctx.org_id())
        .bind(id)
        .fetch_optional(ctx.db.tx_pool())
        .await
        .ok()??;
    let mut params = Map::new();
    params.insert(parent.into(), locator.clone().into());
    params.insert(child.into(), entity_ref.into());
    let text = dispatch(command, Value::Object(params), ctx).await.ok()?;
    let mut state: Value = serde_json::from_str(&text).ok()?;
    state.as_object_mut()?.insert(parent.into(), locator.into());
    Some(state)
}

/// A command output that is the entity itself (most creates and updates
/// echo it), for kinds that have no read command of their own.
fn output_entity(output: &Value, entity_ref: &str) -> Option<Value> {
    (output.get("id").and_then(Value::as_str) == Some(entity_ref)).then(|| output.clone())
}

fn normalize(mut state: Value) -> Value {
    if let Some(object) = state.as_object_mut() {
        object.retain(|key, _| !VOLATILE.contains(&key.as_str()) && key != SECRETS_KEY);
    }
    state
}

/// Compare older journal shapes with today's public projection. Locators are
/// immutable and newly introduced secret markers cannot verify old revisions.
pub(super) fn matches_current(kind: EntityKind, recorded: &Value, current: &Value) -> bool {
    let project = |state: &Value| {
        let mut value = normalize(state.clone());
        if kind == EntityKind::Skill
            && let Some(fields) = value.as_object_mut()
        {
            fields.retain(|key, _| matches!(key.as_str(), "id" | "status" | "skill_md"));
        }
        if kind == EntityKind::Provider {
            value["request_options"] = provider_options(&value["request_options"]);
            value["trace"] = provider_trace(&value["trace"]);
        }
        if kind == EntityKind::AgentChannel {
            let channel_type = value["channel_type"].as_str().unwrap_or("");
            value["channel_config"] = crate::domains::agent_channels::history::public_config(
                channel_type,
                &value["channel_config"],
            );
        }
        if let Some(object) = value.as_object_mut() {
            // Pre-localization revisions omit the newly defaulted empty map.
            // Keep nonempty overlays in the comparison to detect concurrent edits.
            if matches!(kind, EntityKind::Agent | EntityKind::Harness)
                && object
                    .get("localizations")
                    .and_then(Value::as_object)
                    .is_some_and(Map::is_empty)
            {
                object.remove("localizations");
            }
            for locator in restore_locators(kind) {
                object.remove(*locator);
            }
        }
        value
    };
    // Compare the secret markers separately: normalize strips them by design.
    let secrets_match = recorded
        .get(SECRETS_KEY)
        .and_then(Value::as_object)
        .is_none_or(|markers| {
            markers.iter().all(|(key, value)| {
                current.get(SECRETS_KEY).and_then(|all| all.get(key)) == Some(value)
            })
        });
    secrets_match && project(recorded) == project(current)
}

/// Sorted keys, so equal states hash equal whatever order fields came in.
fn canonical(value: &Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut keys: Vec<&String> = object.keys().collect();
            keys.sort();
            Value::Object(
                keys.into_iter()
                    .map(|key| (key.clone(), canonical(&object[key])))
                    .collect(),
            )
        }
        Value::Array(items) => Value::Array(items.iter().map(canonical).collect()),
        other => other.clone(),
    }
}

fn hash(snapshot: &Value) -> String {
    use sha2::{Digest, Sha256};
    let text = canonical(snapshot).to_string();
    hex::encode(Sha256::digest(text.as_bytes()))
}

/// The encrypted columns of `kind`, by field name, as stored now.
async fn secret_columns(
    ctx: &Ctx,
    kind: EntityKind,
    entity_ref: &str,
    state: &Value,
) -> Vec<(&'static str, Option<Vec<u8>>)> {
    let (Some(id), org) = (ref_uuid(entity_ref), ctx.org_id()) else {
        return Vec::new();
    };
    let db = &ctx.db;
    match kind {
        EntityKind::TracingDestination => {
            match db.tracing_destination(org, id).await.ok().flatten() {
                Some(row) => Some(vec![(
                    "authentication",
                    db.tracing_secrets(org, row.connection_id)
                        .await
                        .ok()
                        .flatten(),
                )]),
                None => None,
            }
        }
        EntityKind::Provider => db
            .get_provider(org, id)
            .await
            .ok()
            .flatten()
            .map(|row| vec![("api_key", row.api_key_encrypted)]),
        EntityKind::McpServer => db
            .get_mcp_server(org, id)
            .await
            .ok()
            .flatten()
            .map(|row| vec![("api_key", row.api_key_encrypted)]),
        EntityKind::PaymentAccount => db
            .get_payment_account(org, id)
            .await
            .ok()
            .flatten()
            .map(|row| vec![("credential", row.credential_encrypted)]),
        EntityKind::AgentTrigger => {
            let trigger = everruns_contracts::typed_id::TriggerId::from_uuid(id);
            db.get_agent_trigger(org, trigger)
                .await
                .ok()
                .flatten()
                .map(|row| vec![("config", row.config_encrypted)])
        }
        EntityKind::AgentChannel => {
            let agent = state
                .get("agent_id")
                .and_then(Value::as_str)
                .and_then(ref_uuid);
            match agent {
                Some(agent) => db
                    .get_agent_channel(org, agent, entity_ref)
                    .await
                    .ok()
                    .flatten()
                    .map(|row| {
                        vec![
                            ("channel_config", row.channel_config_encrypted),
                            ("auth", row.auth_encrypted),
                        ]
                    }),
                None => None,
            }
        }
        _ => None,
    }
    .unwrap_or_default()
}

/// One marker per secret column: set or not, and a keyed fingerprint.
pub(super) async fn secret_markers(
    ctx: &Ctx,
    kind: EntityKind,
    entity_ref: &str,
    state: &Value,
) -> Map<String, Value> {
    let mut markers = Map::new();
    for (field, ciphertext) in secret_columns(ctx, kind, entity_ref, state).await {
        let marker = match ciphertext {
            None => serde_json::json!({ "set": false }),
            Some(ciphertext) => {
                let fingerprint = ctx.encryption.as_ref().and_then(|encryption| {
                    let plaintext = encryption.decrypt(&ciphertext).ok()?;
                    Some(encryption.fingerprint(&plaintext))
                });
                serde_json::json!({ "set": true, "fingerprint": fingerprint })
            }
        };
        markers.insert(field.to_string(), marker);
    }
    if kind == EntityKind::Provider
        && let Some(id) = ref_uuid(entity_ref)
        && let Ok(Some(row)) = ctx.db.get_provider(ctx.org_id(), id).await
    {
        let headers = row
            .settings
            .get("request_options")
            .and_then(|value| value.get("headers"))
            .and_then(Value::as_array);
        let set = headers.is_some_and(|headers| !headers.is_empty());
        let fingerprint = ctx.encryption.as_ref().filter(|_| set).map(|encryption| {
            encryption.fingerprint(
                canonical(&serde_json::json!(headers))
                    .to_string()
                    .as_bytes(),
            )
        });
        markers.insert(
            "request_headers".into(),
            serde_json::json!({"set":set,"fingerprint":fingerprint}),
        );
    }
    markers
}

/// One field that differs between two snapshots.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct FieldDiff {
    /// Top-level field name; a secret is `$secrets.<name>`.
    pub field: String,
    /// The value before, `null` when absent. A secret shows only its marker.
    pub from: Value,
    /// The value after, `null` when absent.
    pub to: Value,
}

/// Field-level differences from `from` to `to`, in field order.
pub fn diff(from: &Value, to: &Value) -> Vec<FieldDiff> {
    let empty = Map::new();
    let (from, to) = (
        from.as_object().unwrap_or(&empty),
        to.as_object().unwrap_or(&empty),
    );
    let mut fields: Vec<&String> = from.keys().chain(to.keys()).collect();
    fields.sort();
    fields.dedup();
    let mut changes = Vec::new();
    for field in fields {
        let (before, after) = (from.get(field), to.get(field));
        if field == SECRETS_KEY {
            let none = Value::Object(Map::new());
            changes.extend(
                diff(before.unwrap_or(&none), after.unwrap_or(&none))
                    .into_iter()
                    .map(|change| FieldDiff {
                        field: format!("{SECRETS_KEY}.{}", change.field),
                        ..change
                    }),
            );
        } else if before != after {
            changes.push(FieldDiff {
                field: field.clone(),
                from: before.cloned().unwrap_or(Value::Null),
                to: after.cloned().unwrap_or(Value::Null),
            });
        }
    }
    changes
}

/// The command that makes an entity of `kind` look like a snapshot, and the
/// param naming the entity in it.
pub fn restore_command(kind: EntityKind) -> Option<(&'static str, &'static str)> {
    Some(match kind {
        EntityKind::Organization => ("update_org", "org"),
        EntityKind::OrganizationNetwork => ("set_org_egress_allowlist", "org"),
        EntityKind::OrganizationDecision => ("set_default_decision_model", ""),
        EntityKind::UserProfile => ("update_account_profile", "id"),
        EntityKind::OrganizationAccount => ("update_organization_account_name", "id"),
        EntityKind::TracingPolicy => ("update_tracing_policy", "id"),
        EntityKind::TracingDestination => ("update_tracing_destination_metadata", "id"),
        EntityKind::SandboxTemplate => ("update_sandbox_template_metadata", "id"),
        EntityKind::Agent => ("update_agent", "id"),
        EntityKind::Harness => ("update_harness", "id"),
        EntityKind::Skill => ("update_skill", "id"),
        EntityKind::Capability => ("update_declarative_capability", "id"),
        EntityKind::AgentChannel => ("update_agent_channel", "channel_id"),
        EntityKind::AgentTrigger => ("update_agent_trigger", "trigger_id"),
        EntityKind::AgentScript => ("update_agent_script", "script_id"),
        EntityKind::Schedule => ("update_schedule", "schedule_id"),
        EntityKind::KnowledgeBase => ("update_knowledge_base", "kb_id"),
        EntityKind::KnowledgeEntry => ("update_knowledge_entry", "entry_id"),
        EntityKind::KnowledgeIndex => ("update_knowledge_index", "index_id"),
        EntityKind::Knowledge => ("update_knowledge", "collection_id"),
        EntityKind::KnowledgeIndexer => return None,
        EntityKind::Memory => ("update_memory", "memory_id"),
        EntityKind::Provider => ("update_provider", "id"),
        EntityKind::Model => ("update_model", "id"),
        EntityKind::McpServer => ("update_mcp_server", "id"),
        EntityKind::Plugin => ("update_plugin", "id"),
        EntityKind::PluginMarketplace => ("update_plugin_marketplace", "id"),
        EntityKind::Workspace => ("update_workspace", "workspace_id"),
        EntityKind::VirtualUser => ("update_virtual_user", "id"),
        EntityKind::Observer => ("update_observer", "observer_id"),
        EntityKind::Budget => ("update_budget", "budget_id"),
        EntityKind::PaymentAccount => ("update_payment_account", "payment_account_id"),
        EntityKind::PaymentPolicy => ("update_payment_policy", "payment_policy_id"),
        EntityKind::Eval => ("update_eval", "eval_id"),
        EntityKind::EvalCase => ("update_eval_case", "case_id"),
        EntityKind::SavedReport => ("update_saved_report", "report_id"),
        EntityKind::CheckRule => ("upsert_agent_check_rule", "rule_id"),
        EntityKind::Session => return None,
    })
}

/// Immutable parent identifiers required by nested update commands. They locate
/// the resource and must survive a selected-field restore, never be restored.
pub(super) fn restore_locators(kind: EntityKind) -> &'static [&'static str] {
    match kind {
        EntityKind::AgentChannel
        | EntityKind::AgentTrigger
        | EntityKind::AgentScript
        | EntityKind::CheckRule => &["agent_id"],
        EntityKind::KnowledgeEntry => &["kb_id"],
        EntityKind::EvalCase => &["eval_id"],
        _ => &[],
    }
}

/// The params of an update command's schema.
pub(super) fn update_params(command: &str) -> Option<Map<String, Value>> {
    let descriptor = inventory::iter::<crate::domains::common::CommandDescriptor>
        .into_iter()
        .find(|descriptor| (descriptor.meta)().name == command)?;
    let schema = (descriptor.param_schema)();
    let defs = schema.get("$defs").and_then(Value::as_object);
    let mut params = Map::new();
    crate::services::command_catalog::catalog::collect_all_properties(
        &schema,
        defs,
        &mut params,
        0,
    );
    Some(params)
}

/// `kind`'s update command and the params that make the entity look like
/// `snapshot`: every snapshot field the command takes, never a secret.
pub(super) fn restore_call(
    kind: EntityKind,
    entity_ref: &str,
    snapshot: &Value,
    current: &Value,
) -> Result<(&'static str, Value), CommandError> {
    let unsupported = || {
        CommandError::bad_request(format!(
            "{} entities cannot be restored from history",
            kind.as_str()
        ))
    };
    let (command, id_param) = restore_command(kind).ok_or_else(unsupported)?;
    let accepted = update_params(command).ok_or_else(unsupported)?;
    let mut params: Map<String, Value> = snapshot
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(field, _)| {
            *field != SECRETS_KEY
                && !NEVER_RESTORED.contains(&field.as_str())
                && accepted.contains_key(*field)
        })
        .map(|(field, value)| (field.clone(), value.clone()))
        .collect();
    // A field the revision did not have (reads omit empty optional fields)
    // but the entity has now is cleared, or restore would keep it.
    for (field, value) in current.as_object().into_iter().flatten() {
        if !value.is_null()
            && accepted.contains_key(field)
            && !NEVER_RESTORED.contains(&field.as_str())
            && !VOLATILE.contains(&field.as_str())
            && snapshot.get(field).is_none()
        {
            let empty = if field == "localizations"
                && matches!(kind, EntityKind::Agent | EntityKind::Harness)
            {
                serde_json::json!({})
            } else {
                Value::Null
            };
            params.insert(field.clone(), empty);
        }
    }
    if kind == EntityKind::Provider {
        // Header names/values and credentials come exclusively from CURRENT
        // settings; only the public diagnostics toggle has an inverse.
        let mut options = provider_options(&snapshot["request_options"]);
        options["headers"] = current["request_options"]["headers"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into();
        params.insert("request_options".into(), options);
        params.insert("trace".into(), provider_trace(&snapshot["trace"]));
    }
    for locator in restore_locators(kind) {
        if accepted.contains_key(*locator)
            && let Some(value) = current.get(*locator).or_else(|| snapshot.get(*locator))
        {
            params.insert((*locator).into(), value.clone());
        }
    }
    if kind == EntityKind::AgentChannel {
        let channel_type = current["channel_type"].as_str().unwrap_or("");
        let public = crate::domains::agent_channels::history::public_config(
            channel_type,
            &snapshot["channel_config"],
        );
        params.insert("channel_config".into(), public);
    }
    if !id_param.is_empty() {
        params.insert(id_param.to_string(), entity_ref.into());
    }
    Ok((command, Value::Object(params)))
}

/// Warnings for what a restore keeps as it is now although it differs from
/// the revision: secrets, and fields that may hold them.
pub(super) fn kept_warnings(
    restored: &Value,
    current_state: &Value,
    current: &Map<String, Value>,
    revision: i64,
) -> Vec<String> {
    let mut warnings: Vec<String> = NEVER_RESTORED
        .iter()
        .filter(|field| {
            restored.get(**field).is_some() && restored.get(**field) != current_state.get(**field)
        })
        .map(|field| {
            format!(
                "{field} differs from revision {revision} and was not restored, because it can \
                 hold secrets; change it directly if you need the old value"
            )
        })
        .collect();
    warnings.extend(secret_warnings(restored, current, revision));
    warnings
}

fn secret_warnings(restored: &Value, current: &Map<String, Value>, revision: i64) -> Vec<String> {
    let empty = Map::new();
    let then = restored
        .get(SECRETS_KEY)
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    let mut fields: Vec<&String> = then.keys().chain(current.keys()).collect();
    fields.sort();
    fields.dedup();
    fields
        .into_iter()
        .filter(|field| then.get(*field) != current.get(*field))
        .map(|field| {
            format!(
                "{field} differs from revision {revision} and was kept as it is now; \
                 re-enter it if you need the old value"
            )
        })
        .collect()
}

/// Fields a restore sent that the update did not bring back, such as an
/// optional field its update command cannot clear.
pub(super) fn unrestored(params: &Value, entity: &Value, id_param: &str) -> Vec<String> {
    params
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(field, _)| *field != id_param)
        .filter(|(field, wanted)| {
            let now = entity.get(*field).unwrap_or(&Value::Null);
            match wanted {
                Value::Null => !now.is_null(),
                wanted if field.as_str() == "request_options" => {
                    provider_options(now) != provider_options(wanted)
                }
                wanted if field.as_str() == "trace" => {
                    provider_trace(now) != provider_trace(wanted)
                }
                wanted if field.as_str() == "channel_config" => {
                    let kind = entity["channel_type"].as_str().unwrap_or("");
                    crate::domains::agent_channels::history::public_config(kind, now) != **wanted
                }
                wanted => now != *wanted,
            }
        })
        .map(|(field, _)| field.clone())
        .collect()
}
