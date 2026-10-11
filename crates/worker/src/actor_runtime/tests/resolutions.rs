//! A turn parked on a person (a client tool, an approval, a question) is
//! continued by the resolution the control plane records under it.

use super::*;

/// Park the session's turn on `tool`'s call `call_id`, and return the turn.
async fn park(fixture: &Fixture, worker: &ActorRuntime<TestHosts>) -> OpenTurn {
    fixture.send("Go ahead.").await;
    assert_eq!(run_until_idle(worker).await, 1);
    fixture.assert_idle();
    let turn = fixture
        .open_turn()
        .await
        .expect("the turn stays open, parked");
    assert_eq!(fixture.count("turn.completed").await, 0);
    turn
}

/// Resolve the parked call, run the continuation, and check it finished the
/// parked turn.
async fn resume(
    fixture: &Fixture,
    worker: &ActorRuntime<TestHosts>,
    parked: &OpenTurn,
    call_id: &str,
    tool: &str,
    result: Result<&str, &str>,
) {
    fixture.resolve(call_id, tool, result).await;
    assert_eq!(run_until_idle(worker).await, 1);
    fixture.assert_idle();
    assert!(fixture.open_turn().await.is_none(), "the turn ended");
    let events = fixture.events().await;
    assert_eq!(
        turn_ids(&events, "turn.started"),
        vec![Some(parked.turn_id())]
    );
    assert_eq!(
        turn_ids(&events, "turn.completed"),
        vec![Some(parked.turn_id())],
        "the parked turn completed, no new one started"
    );
    // The resolution belongs to the turn, it is not a new turn's input.
    for event in events
        .iter()
        .filter(|event| event.event_type == "tool.completed")
    {
        assert_eq!(event.context.turn_id, Some(parked.turn_id()));
    }
}

#[tokio::test]
async fn a_client_tool_parks_the_turn_and_its_result_resumes_it() {
    let fixture = Fixture::new(model("Confirmed.", vec![vec![call("c1", "confirm")]])).await;
    let (worker, _) = fixture.worker("w1");
    let parked = park(&fixture, &worker).await;
    assert_eq!(fixture.count("tool.call_requested").await, 1);

    // Another worker continues it.
    let (other, _) = fixture.worker("w2");
    resume(&fixture, &other, &parked, "c1", "confirm", Ok("yes")).await;
    assert_eq!(fixture.answers().await, vec!["Confirmed."]);
}

#[tokio::test]
async fn an_approval_resumes_the_turn_on_approve_and_on_deny() {
    use everruns_contracts::tool_approval_types::APPROVE_TOOL_CALL_TOOL;
    for (decision, answer) in [
        (Ok("approved"), "Approved."),
        (Err("denied by a person"), "Denied."),
    ] {
        let fixture = Fixture::new(model(
            answer,
            vec![vec![call("a1", APPROVE_TOOL_CALL_TOOL)]],
        ))
        .await;
        let (worker, _) = fixture.worker("w1");
        let parked = park(&fixture, &worker).await;
        resume(
            &fixture,
            &worker,
            &parked,
            "a1",
            APPROVE_TOOL_CALL_TOOL,
            decision,
        )
        .await;
        assert_eq!(fixture.answers().await, vec![answer]);
    }
}

#[tokio::test]
async fn an_answered_question_resumes_the_turn() {
    // A question is a client-side pause like any other: the answer is the
    // call's recorded result.
    let fixture = Fixture::new(model("Blue it is.", vec![vec![call("q1", "confirm")]])).await;
    let (worker, _) = fixture.worker("w1");
    let parked = park(&fixture, &worker).await;
    resume(&fixture, &worker, &parked, "q1", "confirm", Ok("blue")).await;
    assert_eq!(fixture.answers().await, vec!["Blue it is."]);
}

#[tokio::test]
async fn a_message_that_reaches_a_parked_turn_resolves_its_pause() {
    let fixture = Fixture::new(model("Moving on.", vec![vec![call("c1", "confirm")]])).await;
    let (worker, _) = fixture.worker("w1");
    let parked = park(&fixture, &worker).await;

    // A message the server marked as a new turn's input (it landed while the
    // turn still ran) reaches the parked turn: the pause is cancelled and the
    // turn goes on to the message, instead of parking again.
    let message = fixture.send("Never mind, skip it.").await;
    assert_eq!(run_until_idle(&worker).await, 1);
    fixture.assert_idle();

    let events = fixture.events().await;
    assert_eq!(
        turn_ids(&events, "turn.started"),
        vec![Some(parked.turn_id())]
    );
    assert_eq!(
        turn_ids(&events, "turn.completed"),
        vec![Some(parked.turn_id())]
    );
    assert_eq!(
        fixture.count("tool.call_requested").await,
        1,
        "not asked again"
    );
    let settled: Vec<String> = events
        .iter()
        .filter(|event| event.event_type == "tool.completed")
        .map(|event| serde_json::to_string(&event.data).unwrap())
        .collect();
    assert_eq!(settled.len(), 1, "{settled:?}");
    assert!(settled[0].contains("cancelled"), "{settled:?}");
    assert_eq!(fixture.answers().await, vec!["Moving on."]);
    let _ = message;
}
