//! Invocation policy for the offline fixture, not production authorization.
//! The server's PendingChange and intent::validate_reason own the real contract.
//! Keep this small supported entity subset explicit; service execution, history
//! transactions, org flags and manager permissions still require live evaluation.

use serde_json::{Value, json};

// Reuse the actual deterministic detector without depending on the server crate.
#[allow(dead_code)]
#[path = "../../../../crates/server/src/credential_shape.rs"]
mod credential_shape;

#[derive(Clone, Debug)]
pub(super) struct Invocation {
    pub params: Value,
    pub reason: Option<String>,
    pub context_revision: Option<i64>,
}

impl Invocation {
    pub fn cli(args: &Value) -> Result<Self, String> {
        let mut params = args.clone();
        let object = params
            .as_object_mut()
            .ok_or("arguments must be an object")?;
        let reason = object.remove("reason");
        let revision = object.remove("context_revision");
        Self::from_parts(params, reason, revision)
    }

    pub fn native(args: &Value) -> Result<Self, String> {
        let object = args.as_object().ok_or("invocation must be an object")?;
        if object
            .keys()
            .any(|key| !matches!(key.as_str(), "params" | "reason" | "context_revision"))
        {
            return Err(
                "native invocation accepts params, reason and context_revision only".into(),
            );
        }
        let params = object
            .get("params")
            .ok_or("native invocation requires params")?
            .clone();
        if params.get("reason").is_some() || params.get("context_revision").is_some() {
            return Err(
                "reason and context_revision are invocation metadata; put them beside params"
                    .into(),
            );
        }
        Self::from_parts(
            params,
            object.get("reason").cloned(),
            object.get("context_revision").cloned(),
        )
    }

    fn from_parts(
        params: Value,
        reason: Option<Value>,
        revision: Option<Value>,
    ) -> Result<Self, String> {
        let reason = reason
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_owned)
                    .ok_or("reason must be a string")
            })
            .transpose()?;
        let context_revision = revision
            .map(|value| value.as_i64().ok_or("context_revision must be an integer"))
            .transpose()?;
        Ok(Self {
            params,
            reason,
            context_revision,
        })
    }

    pub fn metadata(&self) -> Value {
        json!({"reason": self.reason, "context_revision": self.context_revision})
    }

    /// Matches the production reason contract; credential formats use production code.
    pub fn validate_reason(&mut self) -> Result<(), (&'static str, String)> {
        let Some(raw) = self.reason.as_deref() else {
            return Err((
                "reason_required",
                "Agent changes require a reason; retry with reason describing the user request"
                    .into(),
            ));
        };
        let reason = raw.trim();
        let error = if reason.is_empty() {
            Some("Change reason must not be empty".to_owned())
        } else if reason.chars().count() > 1000 {
            Some("Change reason is longer than 1000 characters".to_owned())
        } else if reason
            .chars()
            .any(|ch| ch.is_control() && ch != '\n' && ch != '\t')
        {
            Some("Change reason must not contain control characters".to_owned())
        } else {
            credential_shape::credential_format(reason).map(|label| format!("Change reason looks like it contains {label}; describe the change without the credential"))
        };
        if let Some(error) = error {
            return Err(("invalid_change_reason", error));
        }
        self.reason = Some(reason.to_owned());
        Ok(())
    }
}

/// The fixture's entity mutations use the default agent policy. Advisory calls,
/// task/message traffic and session resources are exempt in production too.
pub(super) fn recorded_mutation(name: &str) -> bool {
    let Some((verb, noun)) = name.split_once('_') else {
        return false;
    };
    let entity = matches!(
        noun,
        "agent"
            | "harness"
            | "skill"
            | "declarative_capability"
            | "agent_channel"
            | "agent_trigger"
            | "agent_script"
            | "schedule"
            | "knowledge_base"
            | "knowledge_entry"
            | "knowledge_index"
            | "memory"
            | "provider"
            | "model"
            | "mcp_server"
            | "plugin_marketplace"
            | "workspace"
            | "virtual_user"
            | "observer"
            | "budget"
            | "payment_account"
            | "payment_policy"
            | "eval"
            | "eval_case"
            | "saved_report"
            | "session"
    );
    (entity
        && matches!(
            verb,
            "create"
                | "update"
                | "upsert"
                | "delete"
                | "destroy"
                | "copy"
                | "import"
                | "fork"
                | "pause"
                | "resume"
                | "disable"
                | "archive"
                | "unarchive"
                | "pin"
                | "unpin"
        ))
        || matches!(
            name,
            "create_agent_credential_binding"
                | "suspend_agent_exposures"
                | "resume_agent_exposures"
                | "restore_entity_revision"
                | "set_manager_context"
                | "append_manager_context"
                | "clear_manager_context"
        )
}

pub(super) fn checks_context(name: &str) -> bool {
    recorded_mutation(name)
        && !name.starts_with("create_")
        && !name.starts_with("copy_")
        && !name.ends_with("_session")
        && !name.ends_with("_knowledge_entry")
        && !name.ends_with("_eval_case")
        && !matches!(
            name,
            "set_manager_context" | "append_manager_context" | "clear_manager_context"
        )
}

/// Prefer the subject's id over a parent id (for example trigger_id over
/// agent_id). This fixture does not resolve production authorization scopes.
pub(super) fn subject_ref(name: &str, params: &Value) -> Option<String> {
    let field = if name.contains("manager_context") || name == "restore_entity_revision" {
        "entity_ref"
    } else if name.ends_with("agent_credential_binding") {
        "agent_id"
    } else {
        match name.split_once('_').map(|(_, noun)| noun) {
            Some("schedule") => "schedule_id",
            Some("agent_channel") => "channel_id",
            Some("agent_trigger") => "trigger_id",
            Some("agent_script") => "script_id",
            Some("knowledge_base") => "kb_id",
            Some("knowledge_entry") => "entry_id",
            Some("knowledge_index") => "index_id",
            Some("memory") => "memory_id",
            Some("workspace") => "workspace_id",
            Some("observer") => "observer_id",
            Some("budget") => "budget_id",
            Some("payment_account") => "payment_account_id",
            Some("payment_policy") => "payment_policy_id",
            Some("eval") => "eval_id",
            Some("eval_case") => "case_id",
            Some("saved_report") => "report_id",
            Some("session") => "session_id",
            _ => "id",
        }
    };
    params.get(field).and_then(Value::as_str).map(str::to_owned)
}

/// Nest the complete canonical payload schema. Local references are rooted at
/// the JSON document, so moving $defs alone would leave broken references.
pub(super) fn native_schema(payload: &Value) -> Value {
    fn rebase(value: &mut Value) {
        match value {
            Value::Object(object) => {
                if let Some(reference) = object.get_mut("$ref")
                    && let Some(fragment) =
                        reference.as_str().and_then(|text| text.strip_prefix('#'))
                    && (fragment.is_empty() || fragment.starts_with('/'))
                {
                    *reference = json!(format!("#/properties/params{fragment}"));
                }
                for child in object.values_mut() {
                    rebase(child);
                }
            }
            Value::Array(array) => {
                for child in array {
                    rebase(child);
                }
            }
            _ => {}
        }
    }
    let mut payload = payload.clone();
    rebase(&mut payload);
    json!({
        "type":"object", "additionalProperties":false, "required":["params"],
        "properties":{
            "params":payload,
            "reason":{"type":"string","description":"Required for agent entity changes. Describe what the user asked to change; never include credentials."},
            "context_revision":{"type":"integer","description":"Manager context revision read before changing an existing entity. Read context first and pass its revision."}
        }
    })
}
