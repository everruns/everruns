//! Observed provider work is checked independently of its event projection.
//! The provider has already started a call when it reports it; cancellation
//! requests stopping further work and cannot undo an external side effect.

use everruns_core::agents_api_store::{ItemKind, ItemState, PolicyStop};
use everruns_core::events::HostedToolCallData;
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{Recorded, Run};
use crate::openai_agents_api::{AgentsApiError, AgentsApiTool};

const CODE: &str = "provider_tool_policy_violation";
const MESSAGE: &str = "The OpenAI Agents API reported work outside Everruns' allowed runtime policy. Everruns stopped this turn and requested cancellation. Provider work may already have occurred; cancellation cannot undo its effects.";

fn subagent(value: &Value) -> bool {
    [
        "/subagent_id",
        "/subagent_turn_id",
        "/turn/subagent_id",
        "/item/subagent_id",
        "/session/subagent_id",
        "/parent_turn_id",
        "/turn/parent_turn_id",
        "/item/parent_turn_id",
    ]
    .iter()
    .any(|pointer| value.pointer(pointer).is_some_and(|v| !v.is_null()))
}

pub(super) fn safe_subagent_identity(raw: &str) -> String {
    let digest = Sha256::digest(raw.as_bytes());
    let digest = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("rejected-{digest}")
}

fn parent(value: &Value) -> Option<&str> {
    [
        "/parent_turn_id",
        "/root_turn_id",
        "/turn/parent_turn_id",
        "/turn/root_turn_id",
    ]
    .iter()
    .find_map(|pointer| value.pointer(pointer).and_then(Value::as_str))
}

fn empty_inventory(value: &Value, key: &str) -> bool {
    value.as_object().is_some_and(|object| {
        object.len() == 1
            && object
                .get(key)
                .and_then(Value::as_array)
                .is_some_and(Vec::is_empty)
    })
}

/// Only the observed MCP text envelope (or its JSON wire encoding) and an
/// exact empty inventory are accepted. Extra content and cursors are data,
/// not an empty inventory, even when the expected array is empty.
fn inventory_output(output: &Value, key: &str) -> bool {
    let decoded;
    let value = if let Some(raw) = output.as_str() {
        let Ok(parsed) = serde_json::from_str::<Value>(raw) else {
            return false;
        };
        decoded = parsed;
        &decoded
    } else {
        output
    };
    if empty_inventory(value, key) {
        return true;
    }
    let Some(object) = value.as_object() else {
        return false;
    };
    if object.keys().any(|k| {
        !matches!(
            k.as_str(),
            "content" | "_meta" | "structuredContent" | "isError"
        )
    }) || object.get("_meta").is_some_and(|v| !v.is_null())
        || object
            .get("structuredContent")
            .is_some_and(|v| !v.is_null())
        || object
            .get("isError")
            .is_some_and(|v| v != &Value::Bool(false))
    {
        return false;
    }
    let Some(content) = object.get("content").and_then(Value::as_array) else {
        return false;
    };
    let [part] = content.as_slice() else {
        return false;
    };
    let Some(part) = part.as_object() else {
        return false;
    };
    if part.len() != 2 || part.get("type").and_then(Value::as_str) != Some("text") {
        return false;
    }
    part.get("text")
        .and_then(Value::as_str)
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .is_some_and(|value| empty_inventory(&value, key))
}

impl Run<'_> {
    pub(super) async fn recover_policy_root(&mut self) -> Result<(), AgentsApiError> {
        if self.runtime_policy_failure().is_none() || self.turn().provider_turn_id.is_some() {
            return Ok(());
        }
        let Some(session) = self.checkpoint.provider_session_id.clone() else {
            return Ok(());
        };
        let mut roots = self.driver.client.list_turns(&session).await?;
        roots.retain(|turn| {
            !subagent(turn)
                && turn
                    .get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| !self.earlier_root(id))
        });
        roots.sort_by_key(|turn| turn.get("created_at").and_then(Value::as_u64));
        if let Some(id) = roots
            .first()
            .and_then(|turn| turn.get("id").and_then(Value::as_str))
        {
            self.adopt_turn(id.to_string()).await?;
        }
        Ok(())
    }

    pub(super) fn runtime_policy_failure(&self) -> Option<&str> {
        self.turn()
            .policy_stop
            .as_ref()
            .filter(|stop| stop.code == CODE)
            .map(|_| MESSAGE)
    }

    async fn refuse_subagent(&mut self, value: &Value) -> Result<bool, AgentsApiError> {
        let id = value
            .pointer("/turn/id")
            .or_else(|| value.get("turn_id"))
            .or_else(|| value.get("id"))
            .and_then(Value::as_str);
        if let Some(id) = id
            && Some(id) != self.turn().provider_turn_id.as_deref()
        {
            let raw = value
                .pointer("/turn/subagent_id")
                .or_else(|| value.get("subagent_id"))
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            self.set_item_state(
                &format!("subagent:{id}"),
                ItemKind::Subagent,
                &safe_subagent_identity(raw),
                ItemState::Open,
            );
        }
        self.refuse_provider_work().await
    }

    async fn refuse_provider_work(&mut self) -> Result<bool, AgentsApiError> {
        if self.turn().policy_stop.is_none() {
            // Persist before cancellation, so recovery cannot resume Act.
            // Never interpolate provider names, arguments, outputs, or errors.
            self.turn_mut().policy_stop = Some(PolicyStop {
                code: CODE.to_string(),
                message: MESSAGE.to_string(),
                replaced: None,
            });
            self.save().await?;
        }
        Ok(false)
    }

    async fn refuse_provider_item(&mut self, item: &Value) -> Result<bool, AgentsApiError> {
        let kind = item.get("type").and_then(Value::as_str).unwrap_or_default();
        if kind.ends_with("_call") && kind != "function_call" {
            let id = item.get("id").and_then(Value::as_str).unwrap_or("missing");
            if !["hosted:", "hosted-done:", "mcp:", "mcp-result:"]
                .iter()
                .any(|prefix| self.turn().items.contains_key(&format!("{prefix}{id}")))
            {
                // Unknown identity/payload stays in the provider trace. A
                // fixed generic record preserves the observed work and its
                // known or explicitly unknown charge without echoing it.
                let digest = Sha256::digest(id.as_bytes());
                let digest = digest
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>();
                let safe_id = format!("rejected-{digest}");
                let priced_kind = if kind == "mcp_call"
                    || everruns_provider::openai_hosted_tools::hosted_call_tool(kind).is_some()
                {
                    kind
                } else {
                    "provider_tool_call"
                };
                // The sanitized accounting marker and stop share one save.
                self.set_item_state(
                    &format!("hosted-done:{safe_id}"),
                    ItemKind::HostedCall,
                    priced_kind,
                    ItemState::Open,
                );
                self.refuse_provider_work().await?;
                self.finish_provider_call(&safe_id, priced_kind).await?;
                return Ok(false);
            }
        }
        self.refuse_provider_work().await
    }

    async fn finish_provider_call(&mut self, id: &str, kind: &str) -> Result<(), AgentsApiError> {
        let event = self.event(
            Some(id),
            HostedToolCallData {
                turn_id: self.request.turn_id,
                call_id: id.to_string(),
                tool_name: "provider_tool".into(),
                status: "failed".into(),
                summary: Some(
                    "The Everruns turn stopped; provider cancellation was requested.".into(),
                ),
            },
        );
        self.complete(
            &format!("hosted-done:{id}"),
            ItemKind::HostedCall,
            kind,
            Recorded::Unverifiable,
            vec![event],
        )
        .await
    }

    pub(super) async fn close_runtime_policy_calls(&mut self) -> Result<(), AgentsApiError> {
        if !self.driver.runtime_policy {
            return Ok(());
        }
        let pending: Vec<(String, String)> = self
            .turn()
            .items
            .iter()
            .filter_map(|(key, item)| {
                if item.kind == ItemKind::McpCall {
                    let id = key.strip_prefix("mcp:")?;
                    return self
                        .turn()
                        .items
                        .get(&format!("mcp-result:{id}"))
                        .is_none_or(|result| result.state != ItemState::Completed)
                        .then(|| (id.to_string(), "mcp_call".to_string()));
                }
                if item.kind != ItemKind::HostedCall {
                    return None;
                }
                if key.starts_with("hosted-done:rejected-") && item.state != ItemState::Completed {
                    return Some((
                        key.strip_prefix("hosted-done:")?.to_string(),
                        item.local_id.clone(),
                    ));
                }
                let id = key.strip_prefix("hosted:")?;
                self.turn()
                    .items
                    .get(&format!("hosted-done:{id}"))
                    .is_none_or(|done| done.state != ItemState::Completed)
                    .then(|| (id.to_string(), "mcp_call".to_string()))
            })
            .collect();
        for (id, kind) in pending {
            self.finish_provider_call(&id, &kind).await?;
        }
        Ok(())
    }

    fn earlier_ancestry(&self, value: &Value) -> bool {
        [
            "/parent_turn_id",
            "/root_turn_id",
            "/turn/parent_turn_id",
            "/turn/root_turn_id",
            "/item/parent_turn_id",
            "/item/root_turn_id",
        ]
        .iter()
        .all(|pointer| {
            value.pointer(pointer).is_none_or(|id| {
                id.is_null() || id.as_str().is_some_and(|id| self.earlier_root(id))
            })
        })
    }

    fn known_earlier_provenance(&self, value: &Value, turn: Option<&str>) -> bool {
        turn.is_some_and(|id| self.earlier_root(id))
            && self.earlier_ancestry(value)
            && ["/turn/id", "/item/turn_id"].iter().all(|pointer| {
                value.pointer(pointer).is_none_or(|id| {
                    id.is_null() || id.as_str().is_some_and(|id| self.earlier_root(id))
                })
            })
    }

    fn current_provenance(&self, value: &Value) -> bool {
        [
            "/turn_id",
            "/turn/id",
            "/root_turn_id",
            "/turn/root_turn_id",
        ]
        .iter()
        .all(|pointer| {
            value.pointer(pointer).is_none_or(|id| {
                id.is_null()
                    || id.as_str().is_some_and(|id| {
                        self.turn()
                            .provider_turn_id
                            .as_deref()
                            .is_none_or(|own| id == own)
                    })
            })
        })
    }

    fn earlier_root(&self, id: &str) -> bool {
        self.turn()
            .prior_provider_turns
            .iter()
            .any(|prior| prior == id)
    }

    fn offered_function(&self, value: &Value) -> bool {
        let Some(name) = value.get("name").and_then(Value::as_str) else {
            return false;
        };
        self.request.config.agent.tools.iter().any(
            |tool| matches!(tool, AgentsApiTool::Function { name: offered, .. } if offered == name),
        )
    }

    fn function_arguments(value: &Value) -> bool {
        value.get("arguments").is_some_and(|arguments| {
            arguments.is_object()
                || arguments
                    .as_str()
                    .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
                    .is_some_and(|v| v.is_object())
        })
    }

    fn allowed_item(&self, item: &Value, terminal: bool) -> bool {
        if subagent(item) || !self.current_provenance(item) {
            return false;
        }
        let id = item
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty());
        match item.get("type").and_then(Value::as_str) {
            Some("message" | "reasoning" | "compaction" | "function_call_output") => true,
            Some("function_call") => {
                id.is_some() && self.offered_function(item) && Self::function_arguments(item)
            }
            Some("mcp_call") => {
                let key = match item.get("name").and_then(Value::as_str) {
                    Some("list_mcp_resources") => "resources",
                    Some("list_mcp_resource_templates") => "resourceTemplates",
                    _ => return false,
                };
                if id.is_none()
                    || item.get("server_label").and_then(Value::as_str) != Some("codex")
                    || !item
                        .get("arguments")
                        .and_then(Value::as_object)
                        .is_some_and(|a| a.is_empty())
                    || item.get("error").is_some_and(|error| !error.is_null())
                {
                    return false;
                }
                match item.get("status").and_then(Value::as_str) {
                    Some("completed") => item
                        .get("output")
                        .is_some_and(|output| inventory_output(output, key)),
                    Some("in_progress" | "queued") if !terminal => {
                        item.get("output").is_none_or(Value::is_null)
                    }
                    _ => false,
                }
            }
            // Unknown kinds can introduce executable provider surfaces, so
            // the production allowlist covers all item kinds, not *_call only.
            _ => false,
        }
    }

    pub(super) async fn check_provider_item(
        &mut self,
        item: &Value,
        terminal: bool,
    ) -> Result<bool, AgentsApiError> {
        if !self.driver.runtime_policy {
            return Ok(true);
        }
        if self.known_earlier_provenance(item, item.get("turn_id").and_then(Value::as_str)) {
            return Ok(true);
        }
        if item
            .get("turn_id")
            .and_then(Value::as_str)
            .is_some_and(|id| Some(id) != self.turn().provider_turn_id.as_deref())
            && self.turn().provider_turn_id.is_some()
        {
            return self.refuse_provider_work().await;
        }
        if !self.allowed_item(item, terminal) {
            return self.refuse_provider_item(item).await;
        }
        Ok(true)
    }

    pub(super) async fn check_provider_event(
        &mut self,
        event: &Value,
    ) -> Result<bool, AgentsApiError> {
        if !self.driver.runtime_policy {
            return Ok(true);
        }
        let turn_id = event
            .get("turn_id")
            .or_else(|| event.pointer("/turn/id"))
            .and_then(Value::as_str);
        if self.known_earlier_provenance(event, turn_id) {
            return Ok(true);
        }
        if subagent(event) {
            if parent(event).is_some_and(|id| self.earlier_root(id))
                && self.earlier_ancestry(event)
                && turn_id != self.turn().provider_turn_id.as_deref()
            {
                return Ok(true);
            }
            return self.refuse_subagent(event).await;
        }
        if turn_id.is_some_and(|id| Some(id) != self.turn().provider_turn_id.as_deref())
            && self.turn().provider_turn_id.is_some()
        {
            return self.refuse_provider_work().await;
        }
        if event
            .pointer("/session/environment")
            .is_some_and(|environment| {
                !environment.is_null()
                    && environment.get("type").and_then(Value::as_str) != Some("none")
            })
        {
            return self.refuse_provider_work().await;
        }
        if !self.current_provenance(event) {
            return self.refuse_provider_work().await;
        }
        if let Some(item) = event.get("item") {
            return self.check_provider_item(item, false).await;
        }
        if event
            .get("type")
            .and_then(Value::as_str)
            .is_some_and(|kind| kind.starts_with("agent.session.environment."))
        {
            return self.refuse_provider_work().await;
        }
        Ok(true)
    }

    pub(super) async fn check_required_actions(
        &mut self,
        value: &Value,
    ) -> Result<bool, AgentsApiError> {
        if !self.driver.runtime_policy {
            return Ok(true);
        }
        if subagent(value) {
            return self.refuse_provider_work().await;
        }
        let environment = value
            .pointer("/session/environment")
            .or_else(|| value.get("environment"));
        if environment.is_some_and(|environment| {
            !environment.is_null()
                && environment.get("type").and_then(Value::as_str) != Some("none")
        }) {
            return self.refuse_provider_work().await;
        }
        let actions = value
            .pointer("/session/required_actions")
            .or_else(|| value.get("required_actions"));
        let Some(actions) = actions else {
            return Ok(true);
        };
        let Some(actions) = actions.as_array() else {
            return self.refuse_provider_work().await;
        };
        for action in actions {
            let turn = action.get("turn_id").and_then(Value::as_str);
            if self.known_earlier_provenance(action, turn) {
                continue;
            }
            if subagent(action)
                || !self.current_provenance(action)
                || turn != self.turn().provider_turn_id.as_deref()
                || turn.is_none()
                || action.get("type").and_then(Value::as_str) != Some("function_call")
                || !self.offered_function(action)
                || action
                    .get("call_id")
                    .and_then(Value::as_str)
                    .is_none_or(|id| id.is_empty())
                || !Self::function_arguments(action)
            {
                return self.refuse_provider_work().await;
            }
        }
        Ok(true)
    }

    pub(super) async fn check_action_boundary(
        &mut self,
        value: &Value,
    ) -> Result<bool, AgentsApiError> {
        if !self.driver.runtime_policy {
            return Ok(true);
        }
        if self.turn().provider_turn_id.is_none() {
            let session = self.provider_session()?;
            let mut roots = self.driver.client.list_turns(&session).await?;
            roots.retain(|turn| {
                !subagent(turn)
                    && turn
                        .get("id")
                        .and_then(Value::as_str)
                        .is_some_and(|id| !self.earlier_root(id))
            });
            roots.sort_by_key(|turn| turn.get("created_at").and_then(Value::as_u64));
            if let Some(id) = roots
                .first()
                .and_then(|turn| turn.get("id").and_then(Value::as_str))
            {
                self.adopt_turn(id.to_string()).await?;
            }
        }
        if !self.check_required_actions(value).await? {
            return Ok(false);
        }
        let (Some(session), Some(turn)) = (
            self.checkpoint.provider_session_id.clone(),
            self.turn().provider_turn_id.clone(),
        ) else {
            return self.refuse_provider_work().await;
        };
        let items = self.driver.client.list_turn_items(&session, &turn).await?;
        self.check_provider_snapshot(&items, false).await
    }

    pub(super) async fn check_provider_snapshot(
        &mut self,
        items: &[Value],
        terminal: bool,
    ) -> Result<bool, AgentsApiError> {
        if !self.driver.runtime_policy {
            return Ok(true);
        }
        let session = self.provider_session()?;
        let turns = self.driver.client.list_turns(&session).await?;
        let own = self.turn().provider_turn_id.as_deref();
        if turns.iter().any(|turn| {
            !subagent(turn)
                && turn
                    .get("id")
                    .and_then(Value::as_str)
                    .is_none_or(|id| Some(id) != own && !self.earlier_root(id))
        }) {
            return self.refuse_provider_work().await;
        }
        let own_created = turns
            .iter()
            .find(|t| t.get("id").and_then(Value::as_str) == own)
            .and_then(|t| t.get("created_at").and_then(Value::as_u64));
        for child in turns.iter().filter(|t| subagent(t)) {
            let attributed = match parent(child) {
                Some(root) => Some(root) == own || !self.earlier_root(root),
                // An orphan child is ambiguous unless the saved timestamp
                // establishes it predates this root turn.
                None => {
                    !matches!((child.get("created_at").and_then(Value::as_u64), own_created), (Some(child), Some(root)) if child < root)
                }
            };
            if attributed {
                return self.refuse_subagent(child).await;
            }
        }
        for item in items {
            if !self.check_provider_item(item, terminal).await? {
                return Ok(false);
            }
        }
        for (key, correlation) in &self.turn().items {
            if correlation.kind == ItemKind::McpCall {
                let id = key
                    .strip_prefix("mcp:")
                    .or_else(|| key.strip_prefix("mcp-result:"));
                if id.is_none_or(|id| {
                    !items.iter().any(|item| {
                        item.get("id").and_then(Value::as_str) == Some(id)
                            && item.get("type").and_then(Value::as_str) == Some("mcp_call")
                            && self.allowed_item(item, terminal)
                    })
                }) {
                    return self.refuse_provider_work().await;
                }
            }
        }
        // Recovery and action boundaries must validate tracked provider work
        // too; an omitted call cannot make an unsafe permissive checkpoint safe.
        for (key, correlation) in &self.turn().items {
            if correlation.kind != ItemKind::HostedCall {
                continue;
            }
            let id = key
                .strip_prefix("hosted:")
                .or_else(|| key.strip_prefix("hosted-done:"));
            if key.starts_with("hosted-done:") && correlation.local_id != "mcp_call"
                || id.is_none_or(|id| {
                    !items.iter().any(|item| {
                        item.get("id").and_then(Value::as_str) == Some(id)
                            && item.get("type").and_then(Value::as_str) == Some("mcp_call")
                            && self.allowed_item(item, terminal)
                    })
                })
            {
                return self.refuse_provider_work().await;
            }
        }
        {
            let current = self.driver.client.retrieve_session(&session).await?;
            if !self.check_required_actions(&current).await? {
                return Ok(false);
            }
        }
        Ok(true)
    }
}
