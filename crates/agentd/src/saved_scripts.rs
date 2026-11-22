//! The agent's saved scripts for the shell's `tools scripts` on a data plane
//! (parity, saved-script triggers): read and saved on the control plane's
//! platform commands server (`injected::PLATFORM_COMMANDS`, operations
//! `scripts` and `save_script`), which keeps them, as a v1 worker's
//! `GrpcSavedScripts` does.
//!
//! Decisions:
//! - The calls name no agent: the control plane finds the agent of the
//!   session the token names, so a script lists or saves the scripts of
//!   the agent whose turn it is.
//! - A list acts for no one. A save is bound to the turn's input message
//!   ([`SavedScriptStore::for_execution`]): its token's `act` names the
//!   person who sent it, from the session's own record, and the control
//!   plane saves as that turn's management user under `AGENT_MANAGE`, and
//!   only when the agent's shell lets it save scripts (THREAT[TM-TOOL-067]).
//!   An unbound store refuses to save.
//! - A trigger's script run needs only the list: its turn runs the script
//!   the way a model's `bash` call would (`core::host::script_run`).

use std::sync::{Arc, Weak};

use async_trait::async_trait;
use everruns_contracts::runtime::capabilities::injected::{
    PLATFORM_COMMAND_TOOL, PLATFORM_COMMANDS,
};
use everruns_contracts::runtime::saved_scripts::{SavedScript, SavedScriptStore};
use everruns_contracts::typed_id::{MessageId, SessionId};
use serde_json::{Value, json};

use crate::mcp::{Acting, ControllerServer};
use crate::providers::OrgMaterial;

const UNBOUND_SAVE: &str =
    "saving a script requires a management-authorized turn; this one has none";

/// Saved scripts of one session's agent.
#[derive(Clone)]
pub(crate) struct ShardSavedScripts {
    material: Weak<dyn OrgMaterial>,
    session: SessionId,
    /// The control plane's platform commands server.
    url: String,
    /// The turn a save is authorized as; `None` until bound.
    input: Option<MessageId>,
}

impl ShardSavedScripts {
    pub(crate) fn new(material: Weak<dyn OrgMaterial>, session: SessionId, url: String) -> Self {
        Self {
            material,
            session,
            url,
            input: None,
        }
    }

    async fn call(
        &self,
        acting: Acting,
        operation: &str,
        arguments: Value,
    ) -> Result<Value, String> {
        let server = ControllerServer::acting(
            Some(&self.material),
            self.session,
            acting,
            PLATFORM_COMMANDS,
            &self.url,
        )
        .ok_or_else(|| "this node holds no session tokens for saved scripts".to_string())?;
        let text = server
            .call_tool(
                PLATFORM_COMMAND_TOOL,
                json!({ "operation": operation, "arguments": arguments }),
            )
            .await?;
        serde_json::from_str(&text)
            .map_err(|_| "the saved scripts answer did not parse".to_string())
    }
}

fn to_script(record: &Value) -> Option<SavedScript> {
    Some(SavedScript {
        name: record["name"].as_str()?.to_string(),
        description: record["description"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        input_schema: record.get("input_schema").filter(|v| !v.is_null()).cloned(),
        body: record["body"].as_str()?.to_string(),
    })
}

#[async_trait]
impl SavedScriptStore for ShardSavedScripts {
    async fn list(&self) -> Result<Vec<SavedScript>, String> {
        let listed = self.call(Acting::NoOne, "scripts", json!({})).await?;
        Ok(listed
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(to_script)
            .collect())
    }

    async fn save(&self, script: SavedScript) -> Result<SavedScript, String> {
        let Some(input) = self.input else {
            return Err(UNBOUND_SAVE.to_string());
        };
        let saved = self
            .call(
                Acting::Message(input),
                "save_script",
                json!({
                    "name": script.name,
                    "description": script.description,
                    "input_schema": script.input_schema,
                    "body": script.body,
                }),
            )
            .await?;
        to_script(&saved).ok_or_else(|| "the control plane answered no script".to_string())
    }

    fn for_execution(&self, input_message_id: uuid::Uuid) -> Option<Arc<dyn SavedScriptStore>> {
        Some(Arc::new(Self {
            input: Some(MessageId::from_uuid(input_message_id)),
            ..self.clone()
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_record_without_a_body_is_not_a_script() {
        assert!(to_script(&json!({ "name": "a" })).is_none());
        let script = to_script(&json!({
            "name": "a", "description": "d", "body": "echo", "input_schema": null,
        }))
        .unwrap();
        assert_eq!(script.name, "a");
        assert!(script.input_schema.is_none());
    }
}
