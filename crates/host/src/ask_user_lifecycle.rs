//! The unattended `ask_user` resolution, split out of `host.rs` to keep that
//! file under the size ratchet (EVE-1057). It is a method on the same
//! `RuntimeSessionLifecycle`, in its own impl block.

use everruns_core::events::{EventContext, EventRequest};
use everruns_provider::typed_id::{MessageId, TurnId};

use crate::host::{RuntimeHostAdapter, RuntimeSessionLifecycle};

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
    ) -> everruns_provider::error::Result<()> {
        for (tool_call_id, arguments) in calls {
            let result = everruns_provider::unattended_ask_user_result(&arguments);
            let data = everruns_core::events::ToolCompletedData::success(
                tool_call_id.clone(),
                everruns_provider::ASK_USER_TOOL_NAME.to_string(),
                vec![everruns_core::message::ContentPart::tool_result_text(
                    &result,
                )],
                None,
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
