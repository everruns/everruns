// The unattended `ask_user` resolution, split out of `host.rs` to keep that
// file under the size ratchet (EVE-1057). It is a method on the same
// `RuntimeSessionLifecycle`, in its own impl block.

use crate::events::{EventContext, EventRequest};
use everruns_contracts::typed_id::{MessageId, TurnId};

use crate::host::runtime_host::{RuntimeHostAdapter, RuntimeSessionLifecycle};

impl<A: RuntimeHostAdapter> RuntimeSessionLifecycle<A> {
    /// Answer `ask_user` calls the client structurally cannot be asked.
    ///
    /// Emitted instead of parking when the session never declared the
    /// `ask_user` hint. Each call is completed with the options the model
    /// itself marked as defaults and `answered_by: "unattended"`, so the model
    /// gets a truthful account in the same turn: it asked, nobody could be
    /// asked, the defaults applied (EVE-1057).
    ///
    /// Not an error, and not a timeout. Telling `unattended` from `timeout` is
    /// how the model learns whether waiting would ever have helped.
    pub async fn resolve_ask_user_unattended(
        &self,
        turn_id: Option<TurnId>,
        input_message_id: MessageId,
        calls: Vec<(String, serde_json::Value)>,
    ) -> everruns_contracts::error::Result<()> {
        let locale = self
            .adapter
            .session_store(self.org_id)
            .get_session(self.session_id)
            .await
            .ok()
            .flatten()
            .and_then(|session| session.locale);
        let registry = self.adapter.capability_registry();
        for (tool_call_id, arguments) in calls {
            let result = everruns_contracts::unattended_ask_user_result(&arguments);
            let data = unattended_completion(
                &registry,
                tool_call_id.clone(),
                &arguments,
                &result,
                locale.as_deref(),
            );
            let context = match turn_id {
                Some(turn_id) => EventContext::turn(turn_id, input_message_id),
                None => EventContext::empty(),
            };
            tracing::info!(
                session_id = %self.session_id,
                %tool_call_id,
                "no client declared it renders questions; answering with declared defaults"
            );
            self.adapter
                .event_emitter()
                .emit(EventRequest::new(self.session_id, context, data))
                .await?;
        }
        Ok(())
    }
}

fn unattended_completion(
    registry: &crate::capabilities::CapabilityRegistry,
    tool_call_id: String,
    arguments: &serde_json::Value,
    result: &serde_json::Value,
    locale: Option<&str>,
) -> crate::events::ToolCompletedData {
    use crate::tool_narration::{
        ToolNarrationContext, ToolNarrationPhase, render_tool_narration_with_locale,
    };
    let call = everruns_contracts::tool_types::ToolCall {
        id: tool_call_id.clone(),
        name: everruns_contracts::ASK_USER_TOOL_NAME.into(),
        arguments: arguments.clone(),
    };
    // Host builds can omit built-ins. Reuse the registered owner when present.
    let narration = registry
        .get("ask_user")
        .and_then(|capability| {
            capability.narrate(
                None,
                &call,
                ToolNarrationPhase::Completed,
                locale,
                ToolNarrationContext::default(),
            )
        })
        .unwrap_or_else(|| {
            render_tool_narration_with_locale(None, &call, ToolNarrationPhase::Completed, locale)
        });
    crate::events::ToolCompletedData::success(
        tool_call_id,
        everruns_contracts::ASK_USER_TOOL_NAME.into(),
        vec![crate::message::ContentPart::tool_result_text(result)],
        None,
    )
    .with_narration(Some(narration))
}

#[cfg(all(test, feature = "builtins"))]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn unattended_completion_preserves_owned_narration() {
        let mut registry = crate::capabilities::CapabilityRegistry::new();
        registry.register(crate::builtins::AskUserCapability::client_side());
        let arguments = json!({"questions":[{"header":"Release scope","question":"PRIVATE_BODY"}]});
        let result = json!({"answers":[{"other_text":"PRIVATE_ANSWER"}]});
        for (locale, expected) in [
            (None, "Asked user: Release scope"),
            (Some("uk-UA"), "Запитав користувача: Release scope"),
        ] {
            let data = unattended_completion(&registry, "call".into(), &arguments, &result, locale);
            assert_eq!(data.narration.as_deref(), Some(expected));
            assert!(data.success);
        }
    }
}
