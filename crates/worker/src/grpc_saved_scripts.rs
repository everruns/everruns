//! The agent's saved scripts for the `tools scripts` shell command, read and
//! written through the control plane's agent-script commands.
//!
//! Decision: the store is bound to a session, and the session's agent is
//! looked up on first use. The shell never names an agent, so a script can
//! only list or save scripts of the agent whose turn it is.

use async_trait::async_trait;
use everruns_contracts::runtime::saved_scripts::{SavedScript, SavedScriptStore};
use everruns_contracts::typed_id::SessionId;
use serde_json::{Value, json};
use tokio::sync::OnceCell;

use crate::grpc_adapters::GrpcAdapter;

const SURFACE: &str = "Saved scripts";

/// Saved scripts of one session's agent.
pub struct GrpcSavedScripts {
    adapter: GrpcAdapter,
    session_id: SessionId,
    agent_id: OnceCell<String>,
}

impl GrpcSavedScripts {
    pub fn new(adapter: GrpcAdapter, session_id: SessionId) -> Self {
        Self {
            adapter,
            session_id,
            agent_id: OnceCell::new(),
        }
    }

    async fn command(&self, name: &str, params: Value) -> Result<Value, String> {
        match self
            .adapter
            .execute_session_command(SURFACE, name, params, Some(self.session_id))
            .await
        {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(error)) => Err(error.message),
            Err(error) => Err(error.to_string()),
        }
    }

    async fn agent_id(&self) -> Result<&str, String> {
        self.agent_id
            .get_or_try_init(|| async {
                let session = self
                    .command(
                        "get_session",
                        json!({ "session_id": self.session_id.to_string() }),
                    )
                    .await?;
                session["agent_id"]
                    .as_str()
                    .map(str::to_string)
                    .ok_or_else(|| "this session has no agent to keep scripts for".to_string())
            })
            .await
            .map(String::as_str)
    }

    async fn records(&self) -> Result<Vec<Value>, String> {
        let agent_id = self.agent_id().await?;
        let listed = self
            .command("list_agent_scripts", json!({ "agent_id": agent_id }))
            .await?;
        Ok(listed.as_array().cloned().unwrap_or_default())
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
impl SavedScriptStore for GrpcSavedScripts {
    async fn list(&self) -> Result<Vec<SavedScript>, String> {
        Ok(self.records().await?.iter().filter_map(to_script).collect())
    }

    async fn save(&self, script: SavedScript) -> Result<SavedScript, String> {
        let existing = self
            .records()
            .await?
            .into_iter()
            .find(|record| record["name"].as_str() == Some(script.name.as_str()));
        let agent_id = self.agent_id().await?;
        let saved = match existing.as_ref().and_then(|r| r["id"].as_str()) {
            Some(script_id) => {
                self.command(
                    "update_agent_script",
                    json!({
                        "agent_id": agent_id,
                        "script_id": script_id,
                        "description": script.description,
                        "input_schema": script.input_schema,
                        "body": script.body,
                    }),
                )
                .await?
            }
            None => {
                self.command(
                    "create_agent_script",
                    json!({
                        "agent_id": agent_id,
                        "name": script.name,
                        "description": script.description,
                        "input_schema": script.input_schema,
                        "body": script.body,
                    }),
                )
                .await?
            }
        };
        to_script(&saved).ok_or_else(|| "the saved script came back malformed".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_map_to_scripts() {
        let record = json!({"id": "scr_1", "name": "triage", "description": "Label PRs.",
                            "body": "echo hi", "agent_id": "agt_1"});
        let script = to_script(&record).unwrap();
        assert_eq!(script.name, "triage");
        assert_eq!(script.input_schema, None);
        assert!(
            to_script(&json!({"name": "x"})).is_none(),
            "a body is required"
        );
    }
}
