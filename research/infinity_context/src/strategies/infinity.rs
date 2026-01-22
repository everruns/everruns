//! Infinity context strategy: trim + history query tool
//!
//! This is the proposed solution that:
//! 1. Keeps recent messages within token budget
//! 2. Injects a notice about excluded messages
//! 3. Provides a tool for the LLM to query historical messages
//!
//! This strategy mirrors the InfinityContextCapability from core, adapted
//! for the evaluation framework.

use super::ContextStrategy;
use crate::types::{
    estimate_tokens, HistoryQuery, Message, MessageExt, MessageRole, PreparedContext, ToolCall,
    ToolDefinition,
};
use anyhow::Result;
use serde_json::json;

pub struct InfinityContextStrategy {
    /// Minimum number of recent messages to always keep
    min_recent: usize,
    /// Boost recency in search results
    boost_recency: bool,
    /// Boost user/assistant messages over tool results
    boost_conversation: bool,
    /// Lines of context around search matches (for future use)
    #[allow(dead_code)]
    context_lines: usize,
}

impl Default for InfinityContextStrategy {
    fn default() -> Self {
        Self {
            min_recent: 10,
            boost_recency: true,
            boost_conversation: true,
            context_lines: 3,
        }
    }
}

impl InfinityContextStrategy {
    #[allow(dead_code)]
    pub fn with_min_recent(mut self, min_recent: usize) -> Self {
        self.min_recent = min_recent;
        self
    }

    /// Create the query_history tool definition
    fn create_history_tool() -> ToolDefinition {
        ToolDefinition {
            name: "query_history".to_string(),
            description: "Search or retrieve messages from earlier in this conversation that are not currently visible. Use this when you need to reference something discussed previously, find specific information mentioned earlier, or understand context from before the visible messages.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search term to find relevant messages. Searches message content."
                    },
                    "message_range": {
                        "type": "object",
                        "properties": {
                            "from": {
                                "type": "integer",
                                "description": "Start index (0-based, oldest message is 0)"
                            },
                            "to": {
                                "type": "integer",
                                "description": "End index (exclusive)"
                            }
                        },
                        "description": "Retrieve messages by position range"
                    },
                    "message_types": {
                        "type": "array",
                        "items": {
                            "type": "string",
                            "enum": ["user", "assistant", "tool_result"]
                        },
                        "description": "Filter by message type"
                    },
                    "limit": {
                        "type": "integer",
                        "default": 20,
                        "description": "Maximum messages to return"
                    }
                }
            }),
        }
    }

    /// Generate the context notice for excluded messages
    fn generate_notice(excluded_count: usize) -> String {
        if excluded_count == 0 {
            return String::new();
        }

        format!(
            "[Context Notice: This conversation has additional history. \
            {} earlier message(s) are not shown to fit context limits. \
            Use the `query_history` tool to search or retrieve earlier messages if needed.]",
            excluded_count
        )
    }

    /// Extract text content from a message
    fn get_text_content(msg: &Message) -> String {
        msg.text_content()
    }

    /// Search excluded messages by keyword
    /// Returns (index, message clone, score) tuples
    fn search_messages(
        &self,
        query: &str,
        messages: &[Message],
        limit: usize,
    ) -> Vec<(usize, Message, f64)> {
        let query_lower = query.to_lowercase();
        let mut results: Vec<(usize, Message, f64)> = Vec::new();

        for (idx, msg) in messages.iter().enumerate() {
            let content = Self::get_text_content(msg);
            let content_lower = content.to_lowercase();

            // Calculate relevance score
            let mut score = 0.0;

            // Keyword match
            if content_lower.contains(&query_lower) {
                score += 1.0;

                // Exact word match bonus
                let words: Vec<&str> = content_lower.split_whitespace().collect();
                if words.contains(&query_lower.as_str()) {
                    score += 0.5;
                }
            }

            if score > 0.0 {
                // Recency boost (newer messages score higher)
                if self.boost_recency && !messages.is_empty() {
                    let recency_factor = (idx as f64 + 1.0) / messages.len() as f64;
                    score += recency_factor * 0.3;
                }

                // Message type boost
                if self.boost_conversation {
                    match msg.role {
                        MessageRole::User | MessageRole::Assistant => score += 0.2,
                        MessageRole::ToolResult => {} // No boost
                        MessageRole::System => score += 0.1,
                    }
                }

                results.push((idx, msg.clone(), score));
            }
        }

        // Sort by score descending
        results.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(limit);
        results
    }

    /// Format search results for return to LLM
    fn format_results(&self, results: &[(usize, Message, f64)], total_messages: usize) -> String {
        if results.is_empty() {
            return "No matching messages found.".to_string();
        }

        let mut output = format!("Found {} relevant message(s) from history:\n\n", results.len());

        for (idx, msg, score) in results {
            let content = Self::get_text_content(msg);
            let truncated_content = if content.len() > 500 {
                format!("{}...", &content[..500])
            } else {
                content
            };

            output.push_str(&format!(
                "--- Message {} of {} ({:?}, relevance: {:.2}) ---\n{}\n\n",
                idx + 1,
                total_messages,
                msg.role,
                score,
                truncated_content
            ));
        }

        output
    }

    /// Get messages by range
    fn get_range(&self, messages: &[Message], from: usize, to: usize, limit: usize) -> String {
        let from = from.min(messages.len());
        let to = to.min(messages.len()).max(from);
        let range_messages: Vec<_> = messages[from..to].iter().take(limit).collect();

        if range_messages.is_empty() {
            return "No messages in the specified range.".to_string();
        }

        let mut output = format!(
            "Messages {} to {} (of {} total):\n\n",
            from + 1,
            from + range_messages.len(),
            messages.len()
        );

        for (offset, msg) in range_messages.iter().enumerate() {
            let content = Self::get_text_content(msg);
            let truncated_content = if content.len() > 500 {
                format!("{}...", &content[..500])
            } else {
                content
            };

            output.push_str(&format!(
                "--- Message {} ({:?}) ---\n{}\n\n",
                from + offset + 1,
                msg.role,
                truncated_content
            ));
        }

        output
    }

    /// Get role string for filtering
    fn role_to_string(role: &MessageRole) -> &'static str {
        match role {
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
            MessageRole::ToolResult => "tool_result",
            MessageRole::System => "system",
        }
    }
}

impl ContextStrategy for InfinityContextStrategy {
    fn name(&self) -> &str {
        "infinity_context"
    }

    fn description(&self) -> &str {
        "Trim messages + provide history query tool (proposed solution)"
    }

    fn prepare_context(&self, messages: &[Message], budget_tokens: usize) -> PreparedContext {
        if messages.is_empty() {
            return PreparedContext {
                messages: vec![],
                excluded_messages: vec![],
                system_additions: vec![],
                additional_tools: vec![Self::create_history_tool()],
                estimated_tokens: 0,
            };
        }

        // Reserve tokens for the history tool definition and potential notice
        let tool_overhead = 500; // Approximate tokens for tool definition
        let notice_overhead = 100; // Approximate tokens for context notice
        let effective_budget = budget_tokens.saturating_sub(tool_overhead + notice_overhead);

        let mut selected: Vec<Message> = Vec::new();
        let mut current_tokens = 0usize;

        // Always include the most recent messages (up to min_recent)
        let recent_start = messages.len().saturating_sub(self.min_recent);
        let recent_messages = &messages[recent_start..];

        for msg in recent_messages {
            selected.push(msg.clone());
            current_tokens += estimate_tokens(msg);
        }

        // Add older messages (newest first) while we have budget
        let older_messages = &messages[..recent_start];
        for msg in older_messages.iter().rev() {
            let msg_tokens = estimate_tokens(msg);
            if current_tokens + msg_tokens > effective_budget {
                break;
            }
            selected.insert(0, msg.clone());
            current_tokens += msg_tokens;
        }

        // Determine excluded messages
        let selected_len = selected.len();
        let excluded: Vec<Message> = messages[..messages.len() - selected_len].to_vec();

        // Generate system additions
        let mut system_additions = Vec::new();
        if !excluded.is_empty() {
            let notice = Self::generate_notice(excluded.len());
            system_additions.push(notice);
            current_tokens += notice_overhead;
        }

        // Add the history tool
        let additional_tools = vec![Self::create_history_tool()];
        current_tokens += tool_overhead;

        PreparedContext {
            messages: selected,
            excluded_messages: excluded,
            system_additions,
            additional_tools,
            estimated_tokens: current_tokens,
        }
    }

    fn handle_tool_call(
        &self,
        tool_call: &ToolCall,
        excluded_messages: &[Message],
    ) -> Result<String> {
        if tool_call.name != "query_history" {
            return Ok(format!("Unknown tool: {}", tool_call.name));
        }

        let query: HistoryQuery = serde_json::from_value(tool_call.arguments.clone())
            .unwrap_or_else(|_| HistoryQuery {
                query: None,
                message_range: None,
                time_range: None,
                message_types: None,
                limit: 20,
            });

        let limit = query.limit.min(50); // Cap at 50

        // Filter by message types if specified
        let filtered_messages: Vec<&Message> = if let Some(ref types) = query.message_types {
            excluded_messages
                .iter()
                .filter(|m| {
                    let role_str = Self::role_to_string(&m.role);
                    types.iter().any(|t| t == role_str)
                })
                .collect()
        } else {
            excluded_messages.iter().collect()
        };

        // Handle range query
        if let Some(range) = query.message_range {
            let owned: Vec<Message> = filtered_messages.into_iter().cloned().collect();
            return Ok(self.get_range(&owned, range.from, range.to, limit));
        }

        // Handle search query
        if let Some(ref search_query) = query.query {
            let owned: Vec<Message> = filtered_messages.into_iter().cloned().collect();
            let results = self.search_messages(search_query, &owned, limit);
            return Ok(self.format_results(&results, excluded_messages.len()));
        }

        // Default: return most recent excluded messages
        let recent: Vec<_> = filtered_messages.iter().rev().take(limit).collect();
        let mut output = format!("Most recent {} excluded messages:\n\n", recent.len());

        for msg in recent.iter() {
            let content = Self::get_text_content(msg);
            let truncated = if content.len() > 500 {
                format!("{}...", &content[..500])
            } else {
                content
            };
            output.push_str(&format!(
                "--- Message ({:?}) ---\n{}\n\n",
                msg.role, truncated
            ));
        }

        Ok(output)
    }

    fn supports_tools(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::message_helpers;

    #[test]
    fn test_no_exclusion_when_under_budget() {
        let strategy = InfinityContextStrategy::default();
        let messages: Vec<Message> = (0..10)
            .map(|i| message_helpers::user(format!("Message {}", i)))
            .collect();

        let result = strategy.prepare_context(&messages, 100000);

        assert_eq!(result.messages.len(), 10);
        assert!(result.excluded_messages.is_empty());
        assert!(result.system_additions.is_empty());
    }

    #[test]
    fn test_exclusion_with_notice() {
        let strategy = InfinityContextStrategy::default().with_min_recent(3);

        let messages: Vec<Message> = (0..50)
            .map(|i| {
                message_helpers::user(format!(
                    "This is message number {} with substantial content",
                    i
                ))
            })
            .collect();

        // Small budget to force exclusion
        let result = strategy.prepare_context(&messages, 200);

        assert!(!result.excluded_messages.is_empty());
        assert!(!result.system_additions.is_empty());
        assert!(result.system_additions[0].contains("query_history"));
    }

    #[test]
    fn test_search_messages() {
        let strategy = InfinityContextStrategy::default();
        let messages = vec![
            message_helpers::user("Let's discuss the API design"),
            message_helpers::assistant("Sure, what about authentication?"),
            message_helpers::user("We should use JWT tokens"),
            message_helpers::assistant("JWT sounds good for the API"),
        ];

        let results = strategy.search_messages("API", &messages, 10);

        assert!(!results.is_empty());
        // Should find messages containing "API"
        assert!(results.iter().any(|(_, m, _)| m.text_content().contains("API")));
    }

    #[test]
    fn test_tool_provides_history_tool() {
        let strategy = InfinityContextStrategy::default();
        let messages: Vec<Message> = (0..5)
            .map(|i| message_helpers::user(format!("Message {}", i)))
            .collect();

        let result = strategy.prepare_context(&messages, 100000);

        assert_eq!(result.additional_tools.len(), 1);
        assert_eq!(result.additional_tools[0].name, "query_history");
    }
}
