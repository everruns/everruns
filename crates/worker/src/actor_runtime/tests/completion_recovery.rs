//! Completing an answer that committed just before its holder died.

use super::*;
use crate::core::RuntimeMessage;
use crate::core::events::{
    EventData, OutputMessageCompletedData, ReasonCompletedData, ReasonStartedData, TokenUsage,
    TurnStartedData,
};

#[tokio::test]
async fn an_answered_turn_retains_its_completion_metadata_after_a_crash() {
    for (tool_rounds, reason_completed) in
        [(false, true), (false, false), (true, true), (true, false)]
    {
        let fixture = Fixture::new(model("Must not run again.", vec![])).await;
        let input = fixture.send("Hello.").await;
        let turn_id = everruns_contracts::typed_id::TurnId::new();
        let context = EventContext::turn(turn_id, input);
        let answer = RuntimeMessage::assistant("Persisted answer.");
        let emitter = fixture.runtime.event_emitter();
        let mut events = vec![EventRequest::new(
            fixture.session_id,
            context.clone(),
            TurnStartedData {
                turn_id,
                input_message_id: input,
                input_content: None,
                agent_id: None,
                agent_name: None,
                agent_description: None,
            },
        )];
        if tool_rounds {
            for count in [2, 1, 1] {
                events.push(EventRequest::new(
                    fixture.session_id,
                    context.clone(),
                    ReasonCompletedData::success(
                        "Tool round.",
                        true,
                        count,
                        Some(10),
                        Some(TokenUsage::new(3, 5)),
                    ),
                ));
            }
        }
        events.extend([
            EventRequest::new(
                fixture.session_id,
                context.clone(),
                ReasonStartedData {
                    harness_id: HarnessId::new(),
                    agent_id: None,
                    metadata: None,
                },
            ),
            EventRequest::new(
                fixture.session_id,
                context.clone(),
                OutputMessageCompletedData::new(answer.clone()),
            ),
        ]);
        if reason_completed {
            events.push(EventRequest::new(
                fixture.session_id,
                context,
                ReasonCompletedData::success(
                    "Persisted answer.",
                    false,
                    0,
                    Some(10),
                    Some(TokenUsage::new(7, 11)),
                ),
            ));
        }
        for event in events {
            emitter.emit(event).await.unwrap();
        }

        let (dead, _) = fixture.worker("dead");
        assert_eq!(dead.lease(1).await.unwrap().len(), 1);
        fixture.store.expire(fixture.session_id);
        let (survivor, _) = fixture.worker("survivor");
        assert_eq!(run_until_idle(&survivor).await, 1);
        let events = fixture.events().await;
        let terminal = events
            .iter()
            .find_map(|event| match &event.data {
                EventData::TurnCompleted(data) => Some(data),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            terminal.final_message_id,
            Some(answer.id),
            "reason completed: {reason_completed}"
        );
        assert_eq!(
            terminal.llm_call_count,
            Some(if tool_rounds { 4 } else { 1 })
        );
        assert_eq!(
            terminal.tool_call_count,
            Some(if tool_rounds { 4 } else { 0 })
        );
        assert_eq!(terminal.issue_count, Some(0));
        assert_eq!(terminal.turn_id, turn_id);
        assert_eq!(terminal.iterations, if tool_rounds { 4 } else { 1 });
        assert_eq!(
            terminal
                .usage
                .as_ref()
                .map(|usage| (usage.input_tokens, usage.output_tokens)),
            reason_completed.then_some((
                7 + if tool_rounds { 9 } else { 0 },
                11 + if tool_rounds { 15 } else { 0 }
            )),
        );
        assert!(terminal.duration_ms.is_some());
        assert_eq!(fixture.model.calls.load(Ordering::SeqCst), 0);
        assert_eq!(fixture.answers().await, vec!["Persisted answer."]);
        assert_eq!(fixture.count("turn.completed").await, 1);
        fixture.assert_idle();
    }
}
