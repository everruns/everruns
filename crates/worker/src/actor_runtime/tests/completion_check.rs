//! The completion check: a finishing turn is reviewed, and an announced but
//! unfinished one continues automatically, once per resume at most.

use everruns_llmsim::{SimToolCall, SimTurn};

use super::*;
use crate::core::completion_check::CompletionCheck;
use crate::core::events::{
    CompletionCheckAction, CompletionVerdict, EventData, TurnCompletionCheckedData,
};

fn tool(name: &str) -> SimTurn {
    SimTurn::ToolCalls(vec![SimToolCall {
        name: name.to_string(),
        arguments: json!({}),
        id: None,
    }])
}

fn say(text: &str) -> SimTurn {
    SimTurn::Assistant(text.to_string())
}

/// The review call's answer.
fn verdict(state: &str, reason: &str) -> SimTurn {
    say(&json!({ "state": state, "reason": reason }).to_string())
}

fn unattended() -> Setup {
    Setup {
        unattended: true,
        ..Setup::default()
    }
}

impl Fixture {
    async fn checks(&self) -> Vec<TurnCompletionCheckedData> {
        self.events()
            .await
            .into_iter()
            .filter_map(|event| match event.data {
                EventData::TurnCompletionChecked(data) => Some(data),
                _ => None,
            })
            .collect()
    }

    /// Automatic continuation inputs, with the turn each was added to.
    async fn continuations(
        &self,
    ) -> Vec<(MessageId, Option<everruns_contracts::typed_id::TurnId>)> {
        self.events()
            .await
            .into_iter()
            .filter_map(|event| match event.data {
                EventData::InputMessage(data) if data.message.is_turn_continuation() => {
                    Some((data.message.id, event.context.turn_id))
                }
                _ => None,
            })
            .collect()
    }

    pub(super) fn model_calls(&self) -> usize {
        self.model.calls.load(Ordering::SeqCst)
    }

    fn review_calls(&self) -> usize {
        self.model
            .seen()
            .iter()
            .filter(|seen| seen.contains("Review the assistant's candidate final"))
            .count()
    }
}

fn actions(
    checks: &[TurnCompletionCheckedData],
) -> Vec<(Option<CompletionVerdict>, CompletionCheckAction, u32)> {
    checks
        .iter()
        .map(|check| (check.verdict, check.action, check.continuations_used))
        .collect()
}

/// The failure this exists for: a review agent says it is submitting the
/// review and ends the turn without the call. The check continues it once,
/// and the continuation makes the call.
#[tokio::test]
async fn an_announced_but_unsubmitted_review_continues_once_and_then_makes_the_call() {
    let fixture = Fixture::with_setup(
        LlmSimConfig::scripted(vec![
            tool("read_inbox"),
            say("Submitting the review now."),
            verdict(
                "in_progress",
                "The review was announced but never submitted.",
            ),
            tool("send_email"),
            say("Review submitted."),
            verdict("achieved", "The review was submitted."),
        ]),
        unattended(),
    )
    .await;
    let (worker, _) = fixture.worker("w1");
    fixture
        .send("Review the pull request and submit the review.")
        .await;

    assert_eq!(run_until_idle(&worker).await, 1);

    assert_eq!(fixture.tool_calls(), vec!["read_inbox", "send_email"]);
    assert_eq!(fixture.model_calls(), 6);
    assert_eq!(fixture.review_calls(), 2);
    let checks = fixture.checks().await;
    assert_eq!(
        actions(&checks),
        vec![
            (
                Some(CompletionVerdict::InProgress),
                CompletionCheckAction::Continued,
                1
            ),
            (
                Some(CompletionVerdict::Achieved),
                CompletionCheckAction::Finished,
                1
            ),
        ]
    );
    assert_eq!(checks[0].max_continuations, 6);
    let continuations = fixture.continuations().await;
    assert_eq!(continuations.len(), 1);
    assert_eq!(checks[0].continuation_message_id, Some(continuations[0].0));
    // One turn: the continuation joined it, it did not start another.
    let turn = turn_ids(&fixture.events().await, "turn.started");
    assert_eq!(turn.len(), 1);
    assert_eq!(continuations[0].1, turn[0]);
    assert_eq!(fixture.count("turn.completed").await, 1);
    assert_eq!(fixture.count("session.idled").await, 1);
    assert_eq!(
        fixture.answers().await,
        vec!["Submitting the review now.", "Review submitted."]
    );
    // The continuation reached the model labeled as the platform's, not the
    // person's.
    assert!(
        fixture.model.seen()[3].contains("[Automatic continuation, not written by the person]")
    );
    fixture.assert_idle();
}

#[tokio::test]
async fn a_completed_turn_is_checked_and_not_continued() {
    let fixture = Fixture::with_setup(
        LlmSimConfig::scripted(vec![
            tool("read_inbox"),
            say("Your inbox is empty."),
            verdict("achieved", "The inbox was checked."),
        ]),
        unattended(),
    )
    .await;
    let (worker, _) = fixture.worker("w1");
    fixture.send("Check my inbox.").await;

    assert_eq!(run_until_idle(&worker).await, 1);

    assert_eq!(fixture.model_calls(), 3);
    assert_eq!(
        actions(&fixture.checks().await),
        vec![(
            Some(CompletionVerdict::Achieved),
            CompletionCheckAction::Finished,
            0
        )]
    );
    assert!(fixture.continuations().await.is_empty());
    assert_eq!(fixture.count("turn.completed").await, 1);
    fixture.assert_idle();
}

/// `off` never checks; `unattended` (the default) does not check a chat
/// session; `always` checks chat too.
#[tokio::test]
async fn the_setting_and_the_session_origin_decide_whether_a_turn_is_checked() {
    for (completion_check, unattended, checked) in [
        (CompletionCheck::Off, true, false),
        (CompletionCheck::Unattended, false, false),
        (CompletionCheck::Always, false, true),
    ] {
        let fixture = Fixture::with_setup(
            LlmSimConfig::scripted(vec![
                tool("read_inbox"),
                say("Submitting the review now."),
                verdict("blocked", "Needs the reviewer's credentials."),
            ]),
            Setup {
                completion_check,
                unattended,
                ..Setup::default()
            },
        )
        .await;
        let (worker, _) = fixture.worker("w1");
        fixture.send("Review the pull request.").await;

        assert_eq!(run_until_idle(&worker).await, 1);

        let case = format!("{completion_check:?}, unattended {unattended}");
        assert_eq!(fixture.review_calls(), usize::from(checked), "{case}");
        assert_eq!(fixture.checks().await.len(), usize::from(checked), "{case}");
        assert!(fixture.continuations().await.is_empty(), "{case}");
        assert_eq!(fixture.count("turn.completed").await, 1, "{case}");
        fixture.assert_idle();
    }
}

#[tokio::test]
async fn an_exhausted_budget_stops_continuing() {
    let mut script = vec![tool("read_inbox")];
    for _ in 0..7 {
        script.push(say("Still working on it."));
        script.push(verdict("in_progress", "Nothing was submitted yet."));
    }
    let fixture = Fixture::with_setup(
        LlmSimConfig::scripted(script),
        Setup {
            max_iterations: 20,
            ..unattended()
        },
    )
    .await;
    let (worker, _) = fixture.worker("w1");
    fixture.send("Review the pull request.").await;

    assert_eq!(run_until_idle(&worker).await, 1);

    let checks = fixture.checks().await;
    let mut expected: Vec<_> = (1..=6)
        .map(|used| {
            (
                Some(CompletionVerdict::InProgress),
                CompletionCheckAction::Continued,
                used,
            )
        })
        .collect();
    expected.push((
        Some(CompletionVerdict::InProgress),
        CompletionCheckAction::BudgetExhausted,
        6,
    ));
    assert_eq!(actions(&checks), expected);
    assert_eq!(fixture.continuations().await.len(), 6);
    assert_eq!(fixture.model_calls(), 15);
    assert_eq!(fixture.count("turn.completed").await, 1);
    fixture.assert_idle();
}

/// A holder that dies after storing the continuation: the next holder reasons
/// on it, without checking that answer again or adding a second one.
#[tokio::test]
async fn a_crash_after_the_continuation_is_stored_resumes_it_without_a_second_one() {
    let fixture = Fixture::with_setup(
        LlmSimConfig::scripted(vec![
            tool("read_inbox"),
            say("Submitting the review now."),
            verdict(
                "in_progress",
                "The review was announced but never submitted.",
            ),
            // Model call 3, the continuation's reason, is held and never
            // reaches the script.
            tool("send_email"),
            say("Review submitted."),
            verdict("achieved", "The review was submitted."),
        ]),
        unattended(),
    )
    .await;
    fixture.model.block(3);
    let (crashing, _) = fixture.worker("w1");
    fixture
        .send("Review the pull request and submit the review.")
        .await;

    crash_at(&fixture, &crashing, &fixture.model.entered).await;
    assert_eq!(fixture.continuations().await.len(), 1);
    assert!(fixture.tool_calls() == vec!["read_inbox"]);

    let (survivor, _) = fixture.worker("w2");
    assert_eq!(run_until_idle(&survivor).await, 1);

    assert_eq!(
        fixture.continuations().await.len(),
        1,
        "no second continuation"
    );
    assert_eq!(
        actions(&fixture.checks().await),
        vec![
            (
                Some(CompletionVerdict::InProgress),
                CompletionCheckAction::Continued,
                1
            ),
            (
                Some(CompletionVerdict::Achieved),
                CompletionCheckAction::Finished,
                1
            ),
        ]
    );
    assert_eq!(fixture.tool_calls(), vec!["read_inbox", "send_email"]);
    assert_eq!(fixture.count("turn.started").await, 1, "the same turn");
    assert_eq!(fixture.count("turn.completed").await, 1);
    fixture.assert_idle();
}

/// A holder that dies during the review: the answer is already stored, so the
/// next holder completes the turn. The check is not run again.
#[tokio::test]
async fn a_crash_during_the_review_completes_the_turn_without_checking_again() {
    let fixture = Fixture::with_setup(
        LlmSimConfig::scripted(vec![
            tool("read_inbox"),
            say("Submitting the review now."),
            verdict("in_progress", "Must not be asked again."),
        ]),
        unattended(),
    )
    .await;
    fixture.model.block(2);
    let (crashing, _) = fixture.worker("w1");
    fixture.send("Review the pull request.").await;

    crash_at(&fixture, &crashing, &fixture.model.entered).await;

    let (survivor, _) = fixture.worker("w2");
    assert_eq!(run_until_idle(&survivor).await, 1);

    assert_eq!(fixture.model_calls(), 3, "the survivor made no model call");
    assert!(fixture.checks().await.is_empty());
    assert!(fixture.continuations().await.is_empty());
    assert_eq!(fixture.count("turn.started").await, 1);
    assert_eq!(fixture.count("turn.completed").await, 1);
    fixture.assert_idle();
}
