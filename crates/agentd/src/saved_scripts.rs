//! The agent's saved scripts for the shell's `tools scripts` on a data plane
//! (parity, saved-script triggers): read and saved through the control
//! plane, which keeps them, as a v1 worker's `GrpcSavedScripts` does.
//!
//! Decisions:
//! - The calls name the session, never an agent: the control plane finds
//!   the session's agent, so a script lists or saves the scripts of the
//!   agent whose turn it is.
//! - A save is bound to the turn's input message
//!   ([`SavedScriptStore::for_execution`]). The forwarding shard names the
//!   person who sent it (`host_calls::act_as_sender`), and the control
//!   plane saves as that turn's management user under `AGENT_MANAGE`, and
//!   only when the agent's shell lets it save scripts (THREAT[TM-TOOL-067]).
//!   An unbound store refuses to save.
//! - A trigger's script run needs only the list: its turn runs the script
//!   the way a model's `bash` call would (`core::host::script_run`).

use std::sync::Arc;

use async_trait::async_trait;
use everruns_contracts::host_calls::{AGENT_SCRIPT_SAVE, AGENT_SCRIPTS};
use everruns_contracts::runtime::saved_scripts::{SavedScript, SavedScriptStore};
use everruns_contracts::typed_id::SessionId;
use everruns_internal_protocol::commands::InternalCommandTransport;
use serde_json::{Value, json};

use crate::lookups::OrgLookups;

const UNBOUND_SAVE: &str =
    "saving a script requires a management-authorized turn; this one has none";

/// Saved scripts of one session's agent.
#[derive(Clone)]
pub(crate) struct ShardSavedScripts {
    lookups: OrgLookups,
    session: SessionId,
    /// The turn a save is authorized as; `None` until bound.
    input: Option<uuid::Uuid>,
}

impl ShardSavedScripts {
    pub(crate) fn new(lookups: OrgLookups, session: SessionId) -> Self {
        Self {
            lookups,
            session,
            input: None,
        }
    }

    async fn call(&self, command: &str, params: Value) -> Result<Value, String> {
        let by_command = everruns_contracts::host_calls::by_name(command)
            .map(|call| call.command)
            .unwrap_or(command);
        match self
            .lookups
            .execute_internal_command(by_command, params)
            .await
        {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(error)) => Err(error.message),
            Err(error) => Err(error.to_string()),
        }
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
        let listed = self
            .call(
                AGENT_SCRIPTS,
                json!({ "session_id": self.session.to_string() }),
            )
            .await?;
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
                AGENT_SCRIPT_SAVE,
                json!({
                    "session_id": self.session.to_string(),
                    "input_message_id": input.to_string(),
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
            input: Some(input_message_id),
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
