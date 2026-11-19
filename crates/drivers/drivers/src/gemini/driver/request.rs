//! One generateContent encoder for interactive and offline requests.
use super::*;

impl GeminiChatDriver {
    pub(super) fn prepare_request(
        &self,
        messages: &[Message],
        config: &LlmCallConfig,
        native_tools: bool,
    ) -> Result<(GeminiRequest, bool)> {
        if is_gemini_4(&config.model)
            && config
                .prompt_cache
                .as_ref()
                .is_some_and(|cache| cache.enabled && cache.gemini_cached_content.is_some())
        {
            return Err(AgentLoopError::config(
                "Gemini 4 models support implicit caching only",
            ));
        }
        let (system_instruction, contents) = Self::convert_messages(messages);

        let (tools, grounded) = match native_tools
            .then(|| Self::native_web_search_tool(config))
            .flatten()
        {
            Some(tool) => (Some(vec![tool]), true),
            None => (Self::convert_tools(&config.tools), false),
        };

        // Reasoning is opt-in on Gemini and silent when absent: without a
        // thinkingConfig the model still reasons but returns no thought parts
        // and no signature, so nothing reaches the reasoning channel and
        // multi-turn tool use loses its thought continuity.
        let thinking_config = config
            .reasoning_effort
            .filter(everruns_contracts::ReasoningEffort::requests_reasoning)
            .map(|effort| GeminiThinkingConfig {
                thinking_budget: (!is_gemini_4(&config.model))
                    .then(|| driver_helpers::thinking_budget::from_effort(effort))
                    .flatten(),
                thinking_level: is_gemini_4(&config.model).then_some(match effort {
                    everruns_contracts::ReasoningEffort::Minimal
                    | everruns_contracts::ReasoningEffort::Low => "low",
                    everruns_contracts::ReasoningEffort::Medium => "medium",
                    // Hydrogen and Neon take `xhigh`; Argon and Carbon answer
                    // it with HTTP 400 (probed 2026-11-19), so they cap at high.
                    everruns_contracts::ReasoningEffort::Xhigh
                    | everruns_contracts::ReasoningEffort::Max
                        if has_xhigh_thinking(&config.model) =>
                    {
                        "xhigh"
                    }
                    _ => "high",
                }),
                include_thoughts: true,
            });

        let mut generation_config = GeminiGenerationConfig {
            temperature: config.temperature.filter(|_| !is_gemini_4(&config.model)),
            max_output_tokens: config.max_tokens,
            thinking_config,
        };

        // If no max_tokens specified, use model's max output from profile, or 8192 fallback
        if generation_config.max_output_tokens.is_none() {
            generation_config.max_output_tokens = Some(if is_gemini_4(&config.model) {
                65_536
            } else {
                everruns_contracts::get_model_profile(
                    &everruns_contracts::DriverId::Gemini,
                    &config.model,
                )
                .and_then(|p| {
                    p.limits
                        .and_then(|l| u32::try_from(l.output).ok().filter(|&v| v > 0))
                })
                .unwrap_or(8_192)
            });
        }

        Ok((
            GeminiRequest {
                contents,
                system_instruction,
                tools,
                generation_config: Some(generation_config),
                cached_content: config
                    .prompt_cache
                    .as_ref()
                    .filter(|cfg| cfg.enabled)
                    .and_then(|cfg| cfg.gemini_cached_content.clone()),
                continuation_token: None,
            },
            grounded,
        ))
    }
}

fn has_xhigh_thinking(model: &str) -> bool {
    matches!(
        model.rsplit('/').next(),
        Some("gemini-4-hydrogen" | "gemini-4-neon")
    )
}
