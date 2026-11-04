// Bedrock Converse prompt caching
//
// Converse caches nothing unless the request carries `cachePoint` blocks, and
// rejects them on models without caching, so cache points go only to the
// families that support them: Claude (system, tools, messages) and Nova
// (system and messages; Nova does not cache tool definitions).
//
// Placement mirrors the Anthropic driver: one point after the system prompt,
// one after the tools, and two on the conversation, four in all, Converse's
// per-request maximum. The pair on the conversation keeps caching incremental:
// a cache point finds an earlier entry only within about 20 content blocks, so
// the point behind the newest one sits where the previous request wrote. The
// explicit strategy drops the conversation points. A 1 hour retention goes
// only to qualified Claude models; other models keep the 5 minute default.

use aws_sdk_bedrockruntime::types::{
    CachePointBlock, CachePointType, CacheTtl, ContentBlock, Message as BedrockMessage,
};
use everruns_contracts::driver_registry::{LlmCallConfig, PromptCacheStrategy};

/// Conversation cache points, after the system and tool points.
const MESSAGE_CACHE_POINTS: usize = 2;

/// Where this request gets cache points.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct CachePlan {
    pub tools: bool,
    pub messages: usize,
    ttl: Option<CacheTtl>,
}

impl CachePlan {
    /// The plan for a call, or `None` when caching is off or the model has no
    /// prompt caching on Bedrock.
    pub fn for_call(config: &LlmCallConfig) -> Option<Self> {
        let cache = config.prompt_cache.as_ref().filter(|cache| cache.enabled)?;
        let model = config.model.to_ascii_lowercase();
        let claude = model.contains("anthropic.claude");
        if !claude && !model.contains("amazon.nova") {
            return None;
        }
        Some(Self {
            tools: claude,
            messages: if cache.strategy == PromptCacheStrategy::Explicit {
                0
            } else {
                MESSAGE_CACHE_POINTS
            },
            ttl: (supports_one_hour(&model) && cache.retention.anthropic_ttl() == Some("1h"))
                .then_some(CacheTtl::OneHour),
        })
    }

    /// A cache point; `None` only if the SDK rejects the block, which it does
    /// solely for a missing type, and the type is always set.
    pub fn point(&self) -> Option<CachePointBlock> {
        CachePointBlock::builder()
            .r#type(CachePointType::Default)
            .set_ttl(self.ttl.clone())
            .build()
            .ok()
    }

    /// Append a cache point to the last `messages` messages.
    pub fn mark_messages(&self, messages: &mut [BedrockMessage]) {
        let Some(point) = self.point() else {
            return;
        };
        let start = messages.len().saturating_sub(self.messages);
        for message in &mut messages[start..] {
            let mut content = message.content().to_vec();
            content.push(ContentBlock::CachePoint(point.clone()));
            if let Ok(marked) = BedrockMessage::builder()
                .role(message.role().clone())
                .set_content(Some(content))
                .build()
            {
                *message = marked;
            }
        }
    }
}

// AWS grants the extended TTL per model, not to the whole Claude family.
// Unknown models keep the default rather than risking a ValidationException.
// https://docs.aws.amazon.com/bedrock/latest/userguide/prompt-caching.html
fn supports_one_hour(model: &str) -> bool {
    let model = model.rsplit('/').next().unwrap_or(model);
    let model = ["us.", "eu.", "apac.", "global."]
        .iter()
        .find_map(|prefix| model.strip_prefix(prefix))
        .unwrap_or(model);
    matches!(
        model,
        "anthropic.claude-haiku-5-5"
            | "anthropic.claude-sonnet-5-5"
            | "anthropic.claude-opus-5-5"
            | "anthropic.claude-fable-5-1"
            | "anthropic.claude-mythos-5-1"
            | "anthropic.claude-fable-5"
            | "anthropic.claude-mythos-5"
            | "anthropic.claude-opus-5"
            | "anthropic.claude-opus-4-8"
            | "anthropic.claude-opus-4-7"
            | "anthropic.claude-opus-4-6-v1"
            | "anthropic.claude-opus-4-5-20251101-v1:0"
            | "anthropic.claude-sonnet-5"
            | "anthropic.claude-sonnet-4-6"
            | "anthropic.claude-haiku-4-5-20251001-v1:0"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use aws_sdk_bedrockruntime::types::ConversationRole;
    use everruns_contracts::driver_registry::{PromptCacheConfig, PromptCacheRetention};

    fn config(
        model: &str,
        strategy: PromptCacheStrategy,
        retention: PromptCacheRetention,
    ) -> LlmCallConfig {
        let mut config = LlmCallConfig::new(model);
        config.prompt_cache = Some(PromptCacheConfig {
            enabled: true,
            strategy,
            retention,
            ..Default::default()
        });
        config
    }

    #[test]
    fn extended_retention_falls_back_for_unqualified_models() {
        for retention in [
            PromptCacheRetention::OneHour,
            PromptCacheRetention::TwelveHours,
        ] {
            for (model, expected) in [
                ("anthropic.claude-sonnet-4-6", Some(&CacheTtl::OneHour)),
                (
                    "us.anthropic.claude-opus-4-5-20251101-v1:0",
                    Some(&CacheTtl::OneHour),
                ),
                (
                    "global.anthropic.claude-sonnet-5-5",
                    Some(&CacheTtl::OneHour),
                ),
                ("anthropic.claude-3-5-sonnet-20241022-v2:0", None),
                ("anthropic.claude-sonnet-4-5-20250929-v1:0", None),
                ("anthropic.claude-unknown", None),
                ("amazon.nova-pro-v1:0", None),
            ] {
                let plan =
                    CachePlan::for_call(&config(model, PromptCacheStrategy::Auto, retention))
                        .unwrap();
                assert_eq!(plan.point().unwrap().ttl(), expected, "{model}");
            }
        }
    }

    #[test]
    fn only_caching_families_get_a_plan() {
        let auto = |model| {
            CachePlan::for_call(&config(
                model,
                PromptCacheStrategy::Auto,
                PromptCacheRetention::Default,
            ))
        };
        let claude = auto("us.anthropic.claude-sonnet-4-5-20250929-v1:0").unwrap();
        assert!(claude.tools);
        assert_eq!(claude.messages, 2);
        let nova = auto("amazon.nova-pro-v1:0").unwrap();
        assert!(!nova.tools, "Nova does not cache tool definitions");
        assert_eq!(auto("meta.llama3-70b-instruct-v1:0"), None);
        let mut off = config(
            "anthropic.claude-sonnet",
            PromptCacheStrategy::Auto,
            PromptCacheRetention::Default,
        );
        off.prompt_cache.as_mut().unwrap().enabled = false;
        assert_eq!(CachePlan::for_call(&off), None);
    }

    #[test]
    fn explicit_drops_conversation_points_and_one_hour_is_model_gated() {
        let explicit = CachePlan::for_call(&config(
            "anthropic.claude-sonnet-4-6",
            PromptCacheStrategy::Explicit,
            PromptCacheRetention::OneHour,
        ))
        .unwrap();
        assert_eq!(explicit.messages, 0);
        assert_eq!(explicit.point().unwrap().ttl(), Some(&CacheTtl::OneHour));
        let nova = CachePlan::for_call(&config(
            "amazon.nova-lite-v1:0",
            PromptCacheStrategy::Auto,
            PromptCacheRetention::OneHour,
        ))
        .unwrap();
        assert_eq!(nova.point().unwrap().ttl(), None);
    }

    #[test]
    fn the_last_messages_end_with_a_cache_point() {
        let plan = CachePlan::for_call(&config(
            "anthropic.claude-sonnet",
            PromptCacheStrategy::Auto,
            PromptCacheRetention::Default,
        ))
        .unwrap();
        let text = |role, text: &str| {
            BedrockMessage::builder()
                .role(role)
                .content(ContentBlock::Text(text.into()))
                .build()
                .unwrap()
        };
        let mut messages = vec![
            text(ConversationRole::User, "one"),
            text(ConversationRole::Assistant, "two"),
            text(ConversationRole::User, "three"),
        ];
        plan.mark_messages(&mut messages);
        let marked: Vec<bool> = messages
            .iter()
            .map(|m| matches!(m.content().last(), Some(ContentBlock::CachePoint(_))))
            .collect();
        assert_eq!(marked, [false, true, true]);
        assert_eq!(messages[2].content()[0], ContentBlock::Text("three".into()));
    }
}
