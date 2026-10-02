//! Observability and cost of the remote loop (EVE-1125): what the managed
//! harness did beyond messages and client functions, projected into the
//! canonical events every SSE, UI, OpenTelemetry, and Braintrust consumer
//! already renders, and what each provider turn cost.
//!
//! - **Subagent turns** become `tool.hosted_call` records named `subagent`.
//!   Their own items never enter the root transcript, and their terminal
//!   events never end the root turn.
//! - **Provider-run tools** (OpenAI-hosted calls) become `tool.hosted_call`.
//! - **Reasoning** becomes `reason.item` with the provider-curated summary
//!   only; hidden reasoning and encrypted context never leave the item.
//! - **Managed compaction** becomes `context.compacted` with the provider as
//!   the strategy; the compacted context itself is never recorded.
//! - **Accounting.** Every provider turn that ends is billed once as an
//!   `llm.generation` (root and each subagent turn separately), whose cost
//!   components cover tokens, per-call tool charges, and hosted containers.
//!   An amount nobody can price is an explicit unknown, never a zero.
//!
//! The backend refuses provider subagents, hosted tools, and environments
//! today (EVE-1124); these mappings stay so lifting a refusal does not leave
//! the work invisible or unbilled.

use std::collections::BTreeMap;

use everruns_core::agents_api_store::{ItemKind, ItemState};
use everruns_core::events::correlation::PROVIDER_SUBAGENT_ID;
use everruns_core::events::{
    CompactionTrigger, ContextCompactedData, EventData, EventRequest, HostedToolCallData,
    LlmCostComponent, LlmGenerationData, ReasonItemData, TokenUsage,
};
use everruns_provider::DriverId;
use everruns_provider::model_profiles::estimate_cost_usd;
use everruns_provider::openai_hosted_tools::{hosted_call_price_usd, hosted_call_tool};
use serde_json::Value;

use super::{Recorded, Run, is_terminal_status};
use crate::openai_agents_api::{
    AgentsApiEnvironment, AgentsApiError, GENERATION_PROVIDER_ID, usage_from,
};

/// Tool name of a provider subagent run on `tool.hosted_call`.
pub(super) const SUBAGENT_TOOL: &str = "subagent";
/// `context.compacted` strategy of provider-managed compaction.
pub(super) const PROVIDER_COMPACTION_STRATEGY: &str = "provider_managed";
/// Container component name of OpenAI-hosted compute.
const OPENAI_CONTAINER: &str = "openai_hosted";

/// A provider item a provider-run tool produced. Everruns only sends client
/// functions, so any other `*_call` item ran inside the managed harness.
pub(super) fn is_hosted_call(kind: &str) -> bool {
    kind.ends_with("_call") && !matches!(kind, "function_call" | "mcp_call")
}

fn hosted_tool_name(kind: &str) -> String {
    hosted_call_tool(kind)
        .map(str::to_string)
        .unwrap_or_else(|| kind.trim_end_matches("_call").to_string())
}

/// How one hosted call bills.
#[derive(Debug, PartialEq)]
enum Charge {
    /// Per call, at the price-table amount when one is known.
    PerCall(Option<f64>),
    /// Per container session: one container serves many calls across turns,
    /// so no per-call amount exists.
    Container,
}

fn charge(kind: &str, model: &str) -> Charge {
    match kind {
        "code_interpreter_call" | "shell_call" => Charge::Container,
        _ => Charge::PerCall(hosted_call_price_usd(kind, model)),
    }
}

/// Usage priced for one provider turn, and its billable components.
///
/// `estimated_cost_usd` carries tokens plus priced per-call tools, but only
/// when the tokens themselves are priced: an unpriced model leaves it unset
/// so budgets fall back to their token-count meter instead of debiting the
/// tools alone. Unknown amounts stay in the components as `None`.
pub(crate) fn turn_cost(
    model: &str,
    usage: Option<TokenUsage>,
    hosted: &BTreeMap<String, u32>,
    container: bool,
) -> (Option<TokenUsage>, Vec<LlmCostComponent>) {
    let mut components = Vec::new();
    let tokens_cost = usage.as_ref().and_then(|usage| {
        estimate_cost_usd(
            &DriverId::OpenAI,
            model,
            usage.input_tokens,
            usage.output_tokens,
            usage.cache_read_tokens.unwrap_or(0),
            usage.cache_creation_tokens.unwrap_or(0),
        )
    });
    components.push(LlmCostComponent::new(
        LlmCostComponent::MODEL_TOKENS,
        model,
        usage.as_ref().map(|usage| {
            u64::from(usage.total_tokens())
                + u64::from(usage.cache_read_tokens.unwrap_or(0))
                + u64::from(usage.cache_creation_tokens.unwrap_or(0))
        }),
        tokens_cost,
    ));
    let mut containers = container;
    let mut tools_cost = 0.0;
    for (kind, count) in hosted {
        match charge(kind, model) {
            Charge::PerCall(price) => {
                let cost = price.map(|price| price * f64::from(*count));
                tools_cost += cost.unwrap_or(0.0);
                components.push(LlmCostComponent::new(
                    LlmCostComponent::HOSTED_TOOL,
                    kind.clone(),
                    Some(u64::from(*count)),
                    cost,
                ));
            }
            Charge::Container => containers = true,
        }
    }
    if containers {
        components.push(LlmCostComponent::new(
            LlmCostComponent::CONTAINER,
            OPENAI_CONTAINER,
            None,
            None,
        ));
    }
    let usage = usage.map(|usage| {
        let estimated = tokens_cost.map(|tokens| tokens + tools_cost);
        usage.with_cost(None, estimated)
    });
    (usage, components)
}

/// Milliseconds between a provider resource's `started_at` and
/// `completed_at` (unix seconds), when both are known.
fn duration_ms(turn: &Value) -> Option<u64> {
    let started = turn.get("started_at").and_then(Value::as_u64)?;
    let completed = turn.get("completed_at").and_then(Value::as_u64)?;
    completed.checked_sub(started).map(|seconds| seconds * 1000)
}

fn id_of(value: &Value) -> Option<&str> {
    value.get("id").and_then(Value::as_str)
}

fn subagent_id_of(turn: &Value) -> Option<&str> {
    turn.get("subagent_id").and_then(Value::as_str)
}

/// The root turn a subagent turn reports, when the provider names one.
fn parent_turn_of(value: &Value) -> Option<&str> {
    [
        "/turn/parent_turn_id",
        "/parent_turn_id",
        "/turn/root_turn_id",
        "/root_turn_id",
    ]
    .iter()
    .find_map(|pointer| value.pointer(pointer).and_then(Value::as_str))
}

impl Run<'_> {
    fn with_subagent(mut event: EventRequest, subagent_id: &str) -> EventRequest {
        if let Some(Value::Object(metadata)) = event.metadata.as_mut() {
            metadata.insert(
                PROVIDER_SUBAGENT_ID.to_string(),
                Value::String(subagent_id.to_string()),
            );
        }
        event
    }

    fn hosted_event(
        &self,
        call_id: &str,
        tool_name: &str,
        status: &str,
        summary: Option<String>,
    ) -> EventRequest {
        self.event(
            Some(call_id),
            HostedToolCallData {
                turn_id: self.request.turn_id,
                call_id: call_id.to_string(),
                tool_name: tool_name.to_string(),
                status: status.to_string(),
                summary,
            },
        )
    }

    /// A provider-run tool call: `in_progress` once, then its end once.
    pub(super) async fn apply_hosted_call(
        &mut self,
        item_id: &str,
        kind: &str,
        item: &Value,
        status: Option<&str>,
    ) -> Result<(), AgentsApiError> {
        let tool = hosted_tool_name(kind);
        let summary = item
            .pointer("/action/query")
            .and_then(Value::as_str)
            .map(str::to_string);
        let start_summary = summary.clone();
        self.open(
            &format!("hosted:{item_id}"),
            ItemKind::HostedCall,
            || item_id.to_string(),
            |run, _| vec![run.hosted_event(item_id, &tool, "in_progress", start_summary)],
        )
        .await?;
        let terminal = match status {
            Some("completed") => "completed",
            Some("failed" | "incomplete" | "cancelled") => "failed",
            _ => return Ok(()),
        };
        let event = self.hosted_event(item_id, &tool, terminal, summary);
        // The kind is kept as the local id so accounting can price the call.
        self.complete(
            &format!("hosted-done:{item_id}"),
            ItemKind::HostedCall,
            kind,
            Recorded::Unverifiable,
            vec![event],
        )
        .await
    }

    /// A reasoning item: identity and the provider-curated summary only.
    pub(super) async fn apply_reasoning(
        &mut self,
        item_id: &str,
        item: &Value,
        status: Option<&str>,
    ) -> Result<(), AgentsApiError> {
        // THREAT[TM-LLM-034]: hidden reasoning (`content`) and opaque replay
        // state (`encrypted_content`) are never read, so they cannot reach
        // the event log; only the safe summary is recorded.
        let summary: Vec<String> = item
            .get("summary")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_string)
            .collect();
        let done = match status {
            Some(status) => status == "completed",
            None => !summary.is_empty(),
        };
        if !done {
            return Ok(());
        }
        let event = self.event(
            Some(item_id),
            ReasonItemData {
                turn_id: self.request.turn_id,
                provider: "openai".to_string(),
                model: Some(self.model()),
                item_id: item_id.to_string(),
                summary,
                token_count: None,
            },
        );
        self.complete(
            &format!("reasoning:{item_id}"),
            ItemKind::Reasoning,
            item_id,
            Recorded::Unverifiable,
            vec![event],
        )
        .await
    }

    /// Provider-managed compaction: recorded as a compaction the provider
    /// performed. The provider holds the compacted context; message counts
    /// are unknown to Everruns and reported as zero on both sides.
    pub(super) async fn apply_compaction(
        &mut self,
        item_id: &str,
        item: &Value,
        status: Option<&str>,
    ) -> Result<(), AgentsApiError> {
        if status.is_some_and(|status| status != "completed") {
            return Ok(());
        }
        let count = |pointer: &str| item.pointer(pointer).and_then(Value::as_u64);
        let event = self.event(
            Some(item_id),
            ContextCompactedData {
                checkpoint_id: None,
                strategy_used: PROVIDER_COMPACTION_STRATEGY.to_string(),
                messages_before: 0,
                messages_after: 0,
                tokens_before: count("/tokens_before").or_else(|| count("/input_tokens")),
                tokens_after: count("/tokens_after").or_else(|| count("/output_tokens")),
                checkpoint_bytes: None,
                replay_source: None,
                bytes_before: None,
                bytes_after: None,
                duration_ms: 0,
                steps: Vec::new(),
                trigger: CompactionTrigger::ContextBudget,
                model: self.model(),
                provider: Some("openai".to_string()),
                driver: Some("openai_agents_api".to_string()),
                budget_remaining_tokens: None,
                source_sequence: None,
                cache_read_tokens: None,
                cache_creation_tokens: None,
            },
        );
        self.complete(
            &format!("compaction:{item_id}"),
            ItemKind::Compaction,
            item_id,
            Recorded::Unverifiable,
            vec![event],
        )
        .await
    }

    /// Key of a subagent run under this root turn: its own turn id, or its
    /// subagent id when the provider reports it under the root turn's id.
    fn subagent_key(&self, value: &Value) -> Option<(String, String)> {
        let own = self.turn().provider_turn_id.as_deref()?;
        let subagent_id = value
            .pointer("/turn/subagent_id")
            .or_else(|| value.get("subagent_id"))
            .and_then(Value::as_str)?
            .to_string();
        if parent_turn_of(value).is_some_and(|parent| parent != own) {
            return None;
        }
        let turn_id = value
            .pointer("/turn/id")
            .or_else(|| value.get("turn_id"))
            .and_then(Value::as_str);
        let key = match turn_id {
            Some(turn_id) if turn_id != own => turn_id.to_string(),
            _ => subagent_id.clone(),
        };
        Some((key, subagent_id))
    }

    async fn open_subagent(&mut self, key: &str, subagent_id: &str) -> Result<(), AgentsApiError> {
        self.open(
            &format!("subagent:{key}"),
            ItemKind::Subagent,
            || subagent_id.to_string(),
            |run, _| {
                vec![Self::with_subagent(
                    run.hosted_event(
                        key,
                        SUBAGENT_TOOL,
                        "in_progress",
                        Some(subagent_id.to_string()),
                    ),
                    subagent_id,
                )]
            },
        )
        .await
        .map(|_| ())
    }

    async fn close_subagent(
        &mut self,
        key: &str,
        subagent_id: &str,
        completed: bool,
    ) -> Result<(), AgentsApiError> {
        self.open_subagent(key, subagent_id).await?;
        let status = if completed { "completed" } else { "failed" };
        let event = Self::with_subagent(
            self.hosted_event(key, SUBAGENT_TOOL, status, Some(subagent_id.to_string())),
            subagent_id,
        );
        self.complete(
            &format!("subagent-done:{key}"),
            ItemKind::Subagent,
            subagent_id,
            Recorded::Unverifiable,
            vec![event],
        )
        .await
    }

    /// A subagent's lifecycle under this root turn. Nothing a subagent emits
    /// sets the root turn terminal or writes into the root transcript.
    pub(super) async fn apply_subagent_event(
        &mut self,
        event_type: &str,
        event: &Value,
    ) -> Result<(), AgentsApiError> {
        let Some((key, subagent_id)) = self.subagent_key(event) else {
            return Ok(());
        };
        match event_type {
            "agent.session.turn.created" => self.open_subagent(&key, &subagent_id).await,
            "agent.session.turn.completed" => self.close_subagent(&key, &subagent_id, true).await,
            "agent.session.turn.failed" | "agent.session.turn.cancelled" => {
                self.close_subagent(&key, &subagent_id, false).await
            }
            _ => Ok(()),
        }
    }

    /// Subagent turns of this root turn: the ones seen live, plus any the
    /// provider lists under this root turn.
    async fn subagent_turns(
        &self,
        session_id: &str,
        own: &str,
    ) -> Result<Vec<Value>, AgentsApiError> {
        let tracked = |id: &str| self.turn().items.contains_key(&format!("subagent:{id}"));
        Ok(self
            .driver
            .client
            .list_turns(session_id)
            .await?
            .into_iter()
            .filter(|turn| subagent_id_of(turn).is_some())
            .filter(|turn| {
                id_of(turn).is_some_and(|id| id != own && tracked(id))
                    || parent_turn_of(turn) == Some(own)
            })
            .collect())
    }

    /// Bill every provider turn of this Everruns turn once: each subagent
    /// turn, then the root turn. Returns the root and subagent usage summed,
    /// or `None` when any of it is unknown (a partial total would read as
    /// the whole).
    ///
    /// `turn` is the root turn resource when the caller already read it;
    /// `poll` re-reads a turn whose usage is still null, since the provider
    /// fills usage after `turn.completed`.
    pub(super) async fn account(
        &mut self,
        turn: Option<Value>,
        final_text: Option<String>,
        poll: bool,
    ) -> Result<Option<TokenUsage>, AgentsApiError> {
        let (Some(session_id), Some(own)) = (
            self.checkpoint.provider_session_id.clone(),
            self.turn().provider_turn_id.clone(),
        ) else {
            return Ok(None);
        };
        let mut turn = match turn {
            Some(turn) => turn,
            None => self.driver.client.retrieve_turn(&session_id, &own).await?,
        };
        let reads = if poll { self.driver.usage_reads } else { 1 };
        for _ in 1..reads {
            if usage_from(&turn).is_some() {
                break;
            }
            tokio::time::sleep(self.driver.usage_backoff).await;
            turn = self.driver.client.retrieve_turn(&session_id, &own).await?;
        }
        let model = self.model();
        let mut total = Some(TokenUsage::default());
        for subagent in self.subagent_turns(&session_id, &own).await? {
            let (Some(sub_turn), Some(subagent_id)) = (
                id_of(&subagent).map(str::to_string),
                subagent_id_of(&subagent).map(str::to_string),
            ) else {
                continue;
            };
            if is_terminal_status(&subagent) {
                let completed = subagent.get("status").and_then(Value::as_str) == Some("completed");
                self.close_subagent(&sub_turn, &subagent_id, completed)
                    .await?;
            }
            let (usage, components) =
                turn_cost(&model, usage_from(&subagent), &BTreeMap::new(), false);
            add_usage(&mut total, usage.as_ref());
            let event = Self::with_subagent(
                self.generation_event(&subagent, usage, components, None, Some(&sub_turn)),
                &subagent_id,
            );
            self.complete(
                &format!("usage:{sub_turn}"),
                ItemKind::Usage,
                &sub_turn,
                Recorded::Unverifiable,
                vec![event],
            )
            .await?;
        }
        let (usage, components) = turn_cost(
            &model,
            usage_from(&turn),
            &self.hosted_counts(),
            !matches!(self.request.config.environment, AgentsApiEnvironment::None),
        );
        add_usage(&mut total, usage.as_ref());
        let event = self.generation_event(&turn, usage, components, final_text, None);
        self.complete(
            &format!("usage:{own}"),
            ItemKind::Usage,
            &own,
            Recorded::Unverifiable,
            vec![event],
        )
        .await?;
        Ok(total)
    }

    /// Hosted calls of this turn that ended, by provider item type.
    fn hosted_counts(&self) -> BTreeMap<String, u32> {
        let mut counts = BTreeMap::new();
        for (key, item) in &self.turn().items {
            if key.starts_with("hosted-done:") && item.state == ItemState::Completed {
                *counts.entry(item.local_id.clone()).or_insert(0) += 1;
            }
        }
        counts
    }

    /// One `llm.generation` for a provider turn, keyed to it by
    /// `response_id`. A turn that did not complete is recorded as a failed
    /// generation that still carries whatever it spent.
    fn generation_event(
        &self,
        provider_turn: &Value,
        usage: Option<TokenUsage>,
        components: Vec<LlmCostComponent>,
        final_text: Option<String>,
        item_id: Option<&str>,
    ) -> EventRequest {
        let model = self.model();
        let provider = self.request.provider.clone();
        let tools = self.request.tools.clone();
        let duration = duration_ms(provider_turn);
        let status = provider_turn
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("cancelled");
        let mut data = if status == "completed" {
            LlmGenerationData::success(
                Vec::new(),
                tools,
                final_text.filter(|text| !text.is_empty()),
                Vec::new(),
                model,
                provider,
                usage,
                duration,
                None,
            )
        } else {
            // A session or environment failure can leave the root turn open.
            let failed = status == "failed" || self.session_failed;
            let message = provider_turn
                .pointer("/error/message")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| {
                    failed
                        .then(|| self.last_error.as_ref().map(|(_, message)| message.clone()))
                        .flatten()
                })
                .unwrap_or_else(|| {
                    if failed {
                        "OpenAI Agents API turn failed".to_string()
                    } else {
                        "OpenAI Agents API turn cancelled".to_string()
                    }
                });
            let mut data = LlmGenerationData::failure(
                Vec::new(),
                tools,
                model,
                provider,
                message,
                duration,
                None,
            );
            data.metadata.usage = usage;
            if !failed {
                data.metadata.finish_reasons = Some(vec!["cancelled".to_string()]);
            }
            data
        };
        data.metadata.response_id = id_of(provider_turn).map(str::to_string);
        let data = data.with_cost_components(components);
        let mut event = self.event(item_id, EventData::LlmGeneration(data));
        // Usage still unknown here is read back later with these credentials
        // (EVE-1145); the id names the provider, never its key.
        if let (Some(Value::Object(metadata)), Some(provider_id)) =
            (event.metadata.as_mut(), self.request.provider_key.as_ref())
        {
            metadata.insert(
                GENERATION_PROVIDER_ID.to_string(),
                Value::String(provider_id.clone()),
            );
        }
        event
    }
}

fn add_usage(total: &mut Option<TokenUsage>, usage: Option<&TokenUsage>) {
    match (total.as_mut(), usage) {
        (Some(total), Some(usage)) => total.add(usage),
        _ => *total = None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage() -> TokenUsage {
        TokenUsage::with_cache(1_000, 100, Some(200), None)
    }

    #[test]
    fn tokens_and_priced_tools_are_billed_and_containers_stay_explicitly_unknown() {
        let model = "gpt-6-astra";
        let (tokens_only, _) = turn_cost(model, Some(usage()), &BTreeMap::new(), false);
        let tokens = tokens_only.unwrap().estimated_cost_usd.unwrap();
        assert!(tokens > 0.0);

        let hosted = BTreeMap::from([
            ("web_search_call".to_string(), 2),
            ("code_interpreter_call".to_string(), 1),
            ("image_generation_call".to_string(), 1),
        ]);
        let (usage, components) = turn_cost(model, Some(usage()), &hosted, false);
        let estimated = usage.unwrap().estimated_cost_usd.unwrap();
        assert!((estimated - (tokens + 0.02)).abs() < 1e-9, "{estimated}");
        let names: Vec<_> = components
            .iter()
            .map(|c| (c.kind.as_str(), c.name.as_str(), c.cost_usd.is_some()))
            .collect();
        assert_eq!(
            names,
            vec![
                ("model_tokens", model, true),
                ("hosted_tool", "image_generation_call", false),
                ("hosted_tool", "web_search_call", true),
                ("container", "openai_hosted", false),
            ]
        );
        assert_eq!(components[0].quantity, Some(1_300));
    }

    #[test]
    fn unknown_usage_is_an_explicit_unknown_not_a_zero() {
        let (usage, components) = turn_cost("gpt-6-astra", None, &BTreeMap::new(), false);
        assert!(usage.is_none());
        assert_eq!(components.len(), 1);
        assert_eq!(components[0].kind, LlmCostComponent::MODEL_TOKENS);
        assert_eq!(components[0].quantity, None);
        assert_eq!(components[0].cost_usd, None);
    }

    #[test]
    fn an_unpriced_model_leaves_the_estimate_to_the_budget_meter() {
        let hosted = BTreeMap::from([("web_search_call".to_string(), 1)]);
        let (usage, components) = turn_cost("no-such-model", Some(usage()), &hosted, false);
        assert_eq!(usage.unwrap().estimated_cost_usd, None);
        assert_eq!(components[0].cost_usd, None);
        assert_eq!(components[1].cost_usd, Some(0.01));
    }

    #[test]
    fn only_provider_run_call_items_are_hosted_calls() {
        assert!(is_hosted_call("web_search_call"));
        assert!(is_hosted_call("code_interpreter_call"));
        assert!(!is_hosted_call("function_call"));
        assert!(!is_hosted_call("mcp_call"));
        assert!(!is_hosted_call("function_call_output"));
        assert_eq!(hosted_tool_name("web_search_call"), "web_search");
        assert_eq!(
            hosted_tool_name("image_generation_call"),
            "image_generation"
        );
        assert_eq!(charge("shell_call", "gpt-6-astra"), Charge::Container);
    }

    #[test]
    fn partial_usage_is_reported_as_unknown() {
        let mut total = Some(TokenUsage::default());
        add_usage(&mut total, Some(&usage()));
        assert_eq!(total.as_ref().unwrap().input_tokens, 1_000);
        add_usage(&mut total, None);
        assert!(total.is_none());
        add_usage(&mut total, Some(&usage()));
        assert!(total.is_none());
    }

    #[test]
    fn duration_comes_from_the_turn_timestamps() {
        let turn = serde_json::json!({"started_at": 10, "completed_at": 13});
        assert_eq!(duration_ms(&turn), Some(3_000));
        assert_eq!(duration_ms(&serde_json::json!({"started_at": 10})), None);
    }
}
