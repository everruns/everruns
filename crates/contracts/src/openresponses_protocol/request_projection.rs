//! One model/endpoint layout decision for serialization and budgeting.
use super::*;
use crate::driver_registry::{LlmCallConfig, Message};

impl OpenResponsesProtocolChatDriver {
    pub(super) fn inline_changes_for(
        &self,
        endpoint: &crate::ProviderEndpoint,
        messages: &[Message],
        config: &LlmCallConfig,
    ) -> Option<super::tool_changes::InlineToolChanges> {
        let mut replaced = Vec::new();
        if self.native_computer_for(config).is_some() {
            replaced.push(crate::native_computer::COMPUTER_TOOL_NAME);
        }
        if self.native_web_search_for(config).is_some() {
            replaced.push(crate::native_web_search::WEB_SEARCH_FUNCTION_TOOL);
        }
        // Hosted search defers tools only at or above its threshold, counted on
        // the baseline that inline mode sends.
        let hosted_search = self.hosted_tool_search
            && config.tool_search.as_ref().is_some_and(|ts| {
                ts.enabled
                    && config.tool_baseline.as_ref().is_some_and(|baseline| {
                        baseline
                            .iter()
                            .filter(|tool| !replaced.contains(&tool.name()))
                            .count()
                            >= ts.threshold
                    })
            });
        super::tool_changes::eligible(
            self.inline_tool_changes,
            endpoint.base_url(),
            config,
            messages,
            &replaced,
        )
        .then_some(super::tool_changes::InlineToolChanges { hosted_search })
    }
}
