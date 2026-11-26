//! Email is an external audience. This policy is installed by the host from
//! the persisted session attachment, never from an incoming message or model.
//! It remains active if MCP discovery fails and is checked on the final tool
//! arguments, including nested and client-executed tool calls.
use crate::tool_context::ToolContext;
use crate::tool_hooks::{PreToolUseDecision, PreToolUseHook};
use everruns_contracts::tool_types::{ToolCall, ToolDefinition};

pub(super) struct EmailReplyPolicy;

// THREAT[TM-EMAIL-002]: untrusted mail cannot widen the channel tool authority.
fn permitted(name: &str, arguments: &serde_json::Value) -> bool {
    // Use the transport's canonical parser: MCP tool names separate server
    // and tool with a double underscore, not a hand-written prefix.
    let allowed = matches!(name, "no_reply" | "send_message")
        || everruns_contracts::runtime::mcp_server::parse_mcp_tool_name(name).is_some_and(
            |(server, tool)| {
                server == "email" && matches!(tool.as_str(), "email_read" | "email_list")
            },
        );
    allowed
        && arguments
            .get("files")
            .is_none_or(|v| v.as_array().is_some_and(Vec::is_empty))
}

#[async_trait::async_trait]
impl PreToolUseHook for EmailReplyPolicy {
    fn is_policy_gate(&self) -> bool {
        true
    }

    async fn before_exec(
        &self,
        call: ToolCall,
        _: &ToolDefinition,
        _: &ToolContext,
    ) -> PreToolUseDecision {
        if permitted(&call.name, &call.arguments) {
            PreToolUseDecision::Continue(call)
        } else {
            PreToolUseDecision::Block {
                tool_call: call,
                reason: "Email permits only scoped email reads and a reply to the current correspondent. Other tools, attachments, and channels require separate authorization.".into(),
                user_message: None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn email_policy_blocks_cross_channel_tools_and_attachments() {
        for name in [
            "slack_send_message",
            "bash",
            "spawn_agent",
            "platform",
            "read_file",
            "query_history",
            "mcp_attacker_email_read",
            "mcp_attacker__email_read",
            "mcp_email_email_read",
        ] {
            assert!(!permitted(name, &json!({})));
        }
        for name in ["send_message", "no_reply"] {
            assert!(permitted(name, &json!({})));
        }
        for tool in ["email_read", "email_list"] {
            let name = everruns_contracts::runtime::mcp_server::mcp_tool_name("email", tool);
            assert!(permitted(&name, &json!({})), "{name}");
        }
        assert!(!permitted(
            "send_message",
            &json!({"files":["/secret.txt"]})
        ));
        assert!(!permitted("send_message", &json!({"files":"/secret.txt"})));
    }
}
