//! The act phase's per-call policy, offered to tools that dispatch a nested
//! tool call (`spawn_background`) through `ToolContext::nested_tool_policy`.
//!
//! Decision (EVE-1186): the nested call runs the same chain functions as a
//! direct call rather than a copy of them, so the hook ordering and gate
//! semantics a direct call gets (including later changes to them) apply to
//! the nested call too. Only capability post-tool hooks run on the nested
//! result: the final hooks bound and persist output for the model's context
//! window, and a background result goes to the run's own artifacts instead.

use std::sync::Arc;

use async_trait::async_trait;

use super::ActAtom;
use crate::engine::phase_effects::PhaseEffectSink;
use crate::engine::tool_context::ToolContext;
use crate::engine::tool_execution::ToolExecutor;
use crate::engine::tool_types::{ToolCall, ToolDefinition, ToolResult};
use crate::tool_hooks::NestedToolPolicy;

use crate::engine::execution::act_hooks::{self, PostToolExecHook, PreToolUseHook};

struct ActNestedToolPolicy {
    outbound_limiter: Option<Arc<dyn crate::tool_execution::OutboundToolRateLimiter>>,
    pre_tool_hooks: Vec<Arc<dyn PreToolUseHook>>,
    post_tool_hooks: Vec<Arc<dyn PostToolExecHook>>,
}

/// Snapshot of `atom`'s chains for one dispatched call.
pub(super) fn for_atom<T, E>(atom: &ActAtom<T, E>) -> Arc<dyn NestedToolPolicy>
where
    T: ToolExecutor,
    E: PhaseEffectSink,
{
    Arc::new(ActNestedToolPolicy {
        outbound_limiter: atom.outbound_tool_rate_limiter.clone(),
        pre_tool_hooks: atom.pre_tool_hooks.clone(),
        post_tool_hooks: atom.post_tool_hooks.clone(),
    })
}

#[async_trait]
impl NestedToolPolicy for ActNestedToolPolicy {
    async fn authorize(
        &self,
        tool_call: ToolCall,
        tool_def: &ToolDefinition,
        context: &ToolContext,
    ) -> Result<ToolCall, ToolResult> {
        // A program must not multiply one admitted outer call into unlimited
        // outbound operations. Reuse the direct call's host-owned limiter.
        if let (Some(limiter), Some(org)) = (&self.outbound_limiter, &context.org_id)
            && !limiter.check_org(org).await
        {
            return Err(crate::tools::ToolExecutionResult::tool_error(
                "Outbound tool rate limit exceeded for this organization; back off and retry later.",
            ).into_tool_result(&tool_call.id, &tool_call.name));
        }
        match act_hooks::pre_tool_use_outcome(&self.pre_tool_hooks, tool_call, tool_def, context)
            .await
        {
            (authorized, None) => Ok(authorized),
            (_, Some(outcome)) => Err(outcome),
        }
    }

    async fn after_exec(
        &self,
        tool_call: &ToolCall,
        tool_def: &ToolDefinition,
        result: &mut ToolResult,
        context: &ToolContext,
    ) {
        act_hooks::run_post_tool_exec_hooks(
            &self.post_tool_hooks,
            &[],
            tool_call,
            tool_def,
            result,
            context,
        )
        .await;
    }

    async fn preview(
        &self,
        tool_call: &ToolCall,
        tool_def: &ToolDefinition,
        context: &ToolContext,
    ) -> Option<ToolResult> {
        for hook in &self.pre_tool_hooks {
            if let Some(held) = hook.preview(tool_call, tool_def, context).await {
                return Some(held);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Deny;
    #[async_trait]
    impl crate::tool_execution::OutboundToolRateLimiter for Deny {
        async fn check_org(&self, _: &crate::typed_id::OrgId) -> bool {
            false
        }
    }
    #[tokio::test]
    async fn nested_calls_share_the_host_outbound_rate_limit() {
        let policy = ActNestedToolPolicy {
            outbound_limiter: Some(Arc::new(Deny)),
            pre_tool_hooks: vec![],
            post_tool_hooks: vec![],
        };
        let mut context = ToolContext::new(crate::typed_id::SessionId::new());
        context.org_id = Some(crate::typed_id::OrgId::new());
        let call = ToolCall {
            id: "nested".into(),
            name: "example".into(),
            arguments: serde_json::json!({}),
        };
        let definition = crate::tool_types::ToolDefinition::function(
            "example",
            "example",
            serde_json::json!({"type":"object"}),
        );
        let result = policy
            .authorize(call, &definition, &context)
            .await
            .unwrap_err();
        assert!(result.error.unwrap().contains("rate limit"));
    }
}
