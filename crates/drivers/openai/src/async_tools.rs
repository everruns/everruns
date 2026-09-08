//! Explicit native async configuration for hosts with a durable call coordinator.

use everruns_provider::{
    LlmCallConfig,
    error::{AgentLoopError, Result},
    native_async::Delivery,
    openresponses_protocol::OpenResponsesRequestExtension,
    tool_types::ToolPolicy,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// The initial supported workload is slow, read-only lookups. The registry's
/// readonly hint is required; authorization must still run at dispatch time.
#[derive(Debug, Clone, Default)]
pub struct NativeAsyncTools {
    /// Tool name -> optional custom format (None keeps a JSON function tool).
    pub tools: BTreeMap<String, Option<Value>>,
    /// Prepared, persisted results from a native-call coordinator.
    pub continuation: Option<Delivery>,
}

impl NativeAsyncTools {
    pub fn function(mut self, name: impl Into<String>) -> Self {
        self.tools.insert(name.into(), None);
        self
    }
    pub fn custom(mut self, name: impl Into<String>, format: Value) -> Self {
        self.tools.insert(name.into(), Some(format));
        self
    }
    pub fn continuation(mut self, delivery: Delivery) -> Self {
        self.continuation = Some(delivery);
        self
    }
}

impl OpenResponsesRequestExtension for NativeAsyncTools {
    fn allow_stateless_recovery(&self) -> bool {
        // Pending calls belong to the provider conversation; replaying a repaired
        // transcript would discard outstanding calls or repeat accepted outputs.
        false
    }
    fn decorate(&self, body: &mut Value, config: &LlmCallConfig) -> Result<()> {
        let supported = config.model == "gpt-6-astra" || config.model.starts_with("gpt-6-astra-");
        if !supported {
            if self.continuation.is_some() {
                return Err(AgentLoopError::config(
                    "cannot switch away from native async while outputs await delivery",
                ));
            }
            // Provider-neutral fallback: the original function definitions remain
            // ordinary synchronous tools. No model defaults are changed.
            return Ok(());
        }
        if self.tools.is_empty() {
            return Ok(());
        }
        if config
            .tool_search
            .as_ref()
            .is_some_and(|search| search.enabled)
        {
            return Err(AgentLoopError::config(
                "native async tools require direct calls; tool search is not enabled for this path",
            ));
        }
        let definitions = body
            .get_mut("tools")
            .and_then(Value::as_array_mut)
            .ok_or_else(|| {
                AgentLoopError::config("native async tools require registered tool definitions")
            })?;
        for (name, format) in &self.tools {
            let def = config
                .tools
                .iter()
                .find(|tool| tool.name() == name)
                .ok_or_else(|| AgentLoopError::config("native async tool is not registered"))?;
            if def.policy() != &ToolPolicy::Auto
                || def.hints().readonly != Some(true)
                || def.hints().concurrency_class.is_some()
            {
                return Err(AgentLoopError::config(
                    "native async initial tools must be automatic, readonly and have no concurrency class",
                ));
            }
            let wire = definitions
                .iter_mut()
                .find(|tool| tool.get("name").and_then(Value::as_str) == Some(name))
                .ok_or_else(|| {
                    AgentLoopError::config(
                        "native async tool must be a direct function or custom tool",
                    )
                })?;
            if let Some(format) = format {
                *wire = json!({"type":"custom", "name":name, "description":def.description(), "format":format});
            }
            wire["async"] = json!(true);
            // No allowed_callers / programmatic execution configuration is emitted.
        }
        // This path is single-agent only. It doesn't set multi-agent options.
        // A caller's parallel_tool_calls=false still controls the local executor.
        if let Some(delivery) = &self.continuation {
            if config.previous_response_id.as_ref() != Some(&delivery.previous_response_id) {
                return Err(AgentLoopError::config(
                    "native continuation must use the latest checkpoint response ID",
                ));
            }
            if config.provider_opaque_context.is_some() {
                return Err(AgentLoopError::config(
                    "native pending outputs cannot cross a compaction boundary",
                ));
            }
            body["previous_response_id"] = json!(delivery.previous_response_id);
            let mut input = delivery.input.clone();
            // Replacing transcript input must retain the current effort transition.
            // Historical transitions already belong to previous_response_id.
            if everruns_provider::reasoning_updates::supports_configuration_updates(&config.model)
                && let Some(effort) = config
                    .reasoning_state
                    .as_ref()
                    .and_then(|state| state.pending)
            {
                input.insert(
                    0,
                    json!({"type":"configuration_update","reasoning":{"effort":effort}}),
                );
            }
            body["input"] = json!(input);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_provider::tool_types::{BuiltinTool, DeferrablePolicy, ToolDefinition, ToolHints};
    fn config(model: &str) -> LlmCallConfig {
        LlmCallConfig {
            speed: None,
            verbosity: None,
            model: model.to_string(),
            temperature: None,
            max_tokens: None,
            tools: vec![],
            reasoning_effort: None,
            reasoning_state: None,
            metadata: std::collections::HashMap::new(),
            previous_response_id: None,
            provider_opaque_context: None,
            tool_search: None,
            prompt_cache: None,
            openrouter_routing: None,
            parallel_tool_calls: None,
            volatile_suffix_len: 0,
            extra_headers: Vec::new(),
            cache_diagnostics: None,
        }
    }

    fn tool(name: &str) -> ToolDefinition {
        ToolDefinition::Builtin(BuiltinTool {
            name: name.into(),
            display_name: None,
            description: "Slow read-only lookup".into(),
            parameters: json!({"type":"object","properties":{"query":{"type":"string"}},"required":["query"]}),
            policy: ToolPolicy::Auto,
            category: None,
            deferrable: DeferrablePolicy::Never,
            hints: ToolHints::default().with_readonly(true),
            full_parameters: None,
        })
    }
    #[test]
    fn opt_in_definitions_and_custom_outputs_follow_original_ids() {
        let mut config = config("gpt-6-astra");
        config.tools = vec![tool("web_fetch"), tool("query")];
        config.previous_response_id = Some("latest".into());
        let delivery = Delivery {
            previous_response_id: "latest".into(),
            call_ids: vec!["old_call".into()],
            input: vec![
                json!({"type":"custom_tool_call_output","call_id":"old_call","output":"answer"}),
            ],
        };
        let options = NativeAsyncTools::default()
            .function("web_fetch")
            .custom("query", json!({"type":"text"}))
            .continuation(delivery.clone());
        let mut body = json!({"tools":[{"type":"function","name":"web_fetch"},{"type":"function","name":"query"}],"input":[{"role":"user","content":"old transcript"}]});
        options.decorate(&mut body, &config).unwrap();
        assert_eq!(body["tools"][0]["async"], true);
        assert_eq!(body["tools"][1]["type"], "custom");
        assert!(body["tools"][1].get("allowed_callers").is_none());
        assert_eq!(body["input"], json!(delivery.input));
        assert_eq!(body["previous_response_id"], "latest");
        config.previous_response_id = Some("stale".into());
        assert!(options.decorate(&mut body, &config).is_err());
    }
    #[test]
    fn continuation_keeps_pending_effort_before_original_call_outputs() {
        use everruns_provider::{ReasoningEffort, reasoning_updates::ReasoningState};
        let mut config = config("gpt-6-astra");
        config.tools = vec![tool("web_fetch")];
        config.previous_response_id = Some("latest".into());
        config.reasoning_state = Some(ReasoningState {
            epoch: "epoch".into(),
            baseline: Some(ReasoningEffort::Low),
            effective: Some(ReasoningEffort::High),
            pending: Some(ReasoningEffort::High),
        });
        let output = json!({"type":"function_call_output","call_id":"original","output":"result"});
        let options = NativeAsyncTools::default()
            .function("web_fetch")
            .continuation(Delivery {
                previous_response_id: "latest".into(),
                input: vec![output.clone()],
                call_ids: vec!["original".into()],
            });
        let mut body = json!({"tools":[{"type":"function","name":"web_fetch"}], "input":[{"type":"message","role":"user","content":"stale transcript"}]});
        options.decorate(&mut body, &config).unwrap();
        assert_eq!(
            body["input"],
            json!([
                {"type":"configuration_update","reasoning":{"effort":"high"}},
                output
            ])
        );
        config.reasoning_state.as_mut().unwrap().pending = None;
        options.decorate(&mut body, &config).unwrap();
        assert_eq!(body["input"], json!([output]));
    }

    #[test]
    fn unsupported_models_fall_back_and_unauthorized_tools_fail() {
        let mut config = config("gpt-5.5");
        config.tools = vec![tool("web_fetch")];
        let options = NativeAsyncTools::default().function("web_fetch");
        let mut body = json!({"tools":[{"type":"function","name":"web_fetch"}]});
        let original = body.clone();
        options.decorate(&mut body, &config).unwrap();
        assert_eq!(body, original);
        config.model = "gpt-6-astra".into();
        if let ToolDefinition::Builtin(tool) = &mut config.tools[0] {
            tool.policy = ToolPolicy::RequiresApproval;
        }
        assert!(options.decorate(&mut body, &config).is_err());
        if let ToolDefinition::Builtin(tool) = &mut config.tools[0] {
            tool.policy = ToolPolicy::Auto;
            tool.hints.readonly = Some(false);
        }
        assert!(options.decorate(&mut body, &config).is_err());
    }
}
