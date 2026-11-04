//! A provider refuses a request for lack of credits: the engine retries
//! smaller once, or waits for credits on durable checks, resumes once, and
//! stands down when the wait runs out.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use everruns_contracts::error::AgentLoopError;
use everruns_contracts::message_origin::MessageOrigin;
use everruns_contracts::provider_limit::CreditStatus;
use everruns_contracts::runtime::session_schedule::SessionSchedule;
use everruns_contracts::runtime::session_services::SessionScheduleStore;
use everruns_contracts::typed_id::{PrincipalId, ScheduleId};

use super::*;
use crate::core::RuntimeMessage;
use crate::core::events::{
    CreditShortfallAction, EventData, InputMessageData, TurnCreditShortfallData,
};
use crate::engine::credit_shortfall::{self, PROBE_INTERVAL_SECS, WAIT_WINDOW_SECS};

/// OpenRouter's refusal when the balance covers fewer tokens than asked.
fn refusal(affordable: u64) -> AgentLoopError {
    let body = format!(
        r#"{{"error":{{"message":"This request requires more credits, or fewer max_tokens. You requested up to 65536 tokens, but can only afford {affordable}. To increase, visit https://openrouter.ai/settings/credits","code":402}}}}"#
    );
    AgentLoopError::llm_http(402, &body, format!("OpenRouter API error (402): {body}"))
}

/// Session schedules in memory; a fire is the test's to make.
#[derive(Default)]
struct Schedules(Mutex<Vec<SessionSchedule>>);

#[async_trait]
impl SessionScheduleStore for Schedules {
    async fn create_schedule(
        &self,
        session_id: SessionId,
        description: String,
        cron_expression: Option<String>,
        scheduled_at: Option<DateTime<Utc>>,
        timezone: String,
    ) -> everruns_contracts::error::Result<SessionSchedule> {
        let now = Utc::now();
        let schedule = SessionSchedule {
            id: ScheduleId::new(),
            session_id,
            owner_principal_id: PrincipalId::new(),
            resolved_owner_user_id: None,
            owner: None,
            effective_owner: None,
            description,
            schedule_type: SessionSchedule::derive_type(&cron_expression),
            cron_expression,
            scheduled_at,
            timezone,
            enabled: true,
            next_trigger_at: scheduled_at,
            last_triggered_at: None,
            trigger_count: 0,
            created_at: now,
            updated_at: now,
            deleted_at: None,
            ends_at: None,
            max_runs: None,
            stop_when: None,
            stop_reason: None,
            budget: None,
        };
        self.0.lock().unwrap().push(schedule.clone());
        Ok(schedule)
    }

    async fn cancel_schedule(
        &self,
        _session_id: SessionId,
        schedule_id: ScheduleId,
    ) -> everruns_contracts::error::Result<SessionSchedule> {
        let mut rows = self.0.lock().unwrap();
        let row = rows.iter_mut().find(|row| row.id == schedule_id).unwrap();
        row.enabled = false;
        row.stop_reason = Some("cancelled".to_string());
        Ok(row.clone())
    }

    async fn list_schedules(
        &self,
        _session_id: SessionId,
    ) -> everruns_contracts::error::Result<Vec<SessionSchedule>> {
        Ok(self.0.lock().unwrap().clone())
    }

    async fn count_active_schedules(
        &self,
        _session_id: SessionId,
    ) -> everruns_contracts::error::Result<u32> {
        Ok(self.0.lock().unwrap().iter().filter(|s| s.enabled).count() as u32)
    }

    async fn count_active_org_schedules(&self) -> everruns_contracts::error::Result<u32> {
        self.count_active_schedules(SessionId::new()).await
    }
}

impl Schedules {
    /// Schedules still waiting to fire.
    fn queued(&self) -> Vec<SessionSchedule> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|s| s.enabled && s.trigger_count == 0)
            .cloned()
            .collect()
    }

    /// Claim the one queued schedule, as the scheduler does on its fire.
    fn claim(&self) -> SessionSchedule {
        let mut rows = self.0.lock().unwrap();
        let mut queued = rows
            .iter_mut()
            .filter(|s| s.enabled && s.trigger_count == 0);
        let row = queued.next().expect("a queued check");
        assert!(queued.next().is_none(), "one queued check at a time");
        row.enabled = false;
        row.trigger_count = 1;
        row.clone()
    }
}

async fn fixture(schedules: &Arc<Schedules>) -> Fixture {
    let store: Arc<dyn SessionScheduleStore> = schedules.clone();
    Fixture::with_setup(
        LlmSimConfig::fixed("Review submitted."),
        Setup {
            schedules: Some(Arc::new(move |_org| store.clone())),
            ..Setup::default()
        },
    )
    .await
}

impl Fixture {
    async fn credit_events(&self) -> Vec<TurnCreditShortfallData> {
        self.events()
            .await
            .into_iter()
            .filter_map(|event| match event.data {
                EventData::TurnCreditShortfall(data) => Some(data),
                _ => None,
            })
            .collect()
    }

    async fn credit_actions(&self) -> Vec<CreditShortfallAction> {
        self.credit_events()
            .await
            .iter()
            .map(|data| data.action)
            .collect()
    }

    /// Fire a schedule: its prompt becomes the session's input, as the
    /// session scheduler stores it.
    async fn fire(&self, schedule: &SessionSchedule) {
        let mut message = RuntimeMessage::user(&schedule.description);
        message.metadata = Some(
            MessageOrigin::schedule(schedule.id.to_string(), &schedule.description).into_metadata(),
        );
        let message_id = message.id;
        self.runtime
            .event_emitter()
            .emit(EventRequest::new(
                self.session_id,
                EventContext::empty(),
                InputMessageData::new(message),
            ))
            .await
            .expect("scheduled message stored");
        self.mark(message_id).await;
    }

    fn refuse(&self, error: AgentLoopError) {
        self.model.refusals.lock().unwrap().push_back(error);
    }

    fn balances(&self, balances: &[Option<CreditStatus>]) {
        self.model
            .balances
            .lock()
            .unwrap()
            .extend(balances.iter().copied());
    }

    fn max_tokens(&self) -> Vec<Option<u32>> {
        self.model.max_tokens.lock().unwrap().clone()
    }
}

/// The incident this exists for, with a balance that still covers a useful
/// answer: the same request is retried once for what the balance covers.
#[tokio::test]
async fn a_balance_that_covers_a_useful_answer_is_retried_smaller_once() {
    let schedules = Arc::new(Schedules::default());
    let fixture = fixture(&schedules).await;
    fixture.refuse(refusal(20_000));
    let (worker, _) = fixture.worker("w1");
    fixture.send("Review the pull request.").await;

    assert_eq!(run_until_idle(&worker).await, 1);

    let max = 20_000 - 512;
    assert_eq!(fixture.max_tokens(), vec![None, Some(max)]);
    assert_eq!(fixture.answers().await, vec!["Review submitted."]);
    let events = fixture.credit_events().await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].action, CreditShortfallAction::Shrunk);
    assert_eq!(events[0].requested_max_tokens, Some(65_536));
    assert_eq!(events[0].affordable_tokens, Some(20_000));
    assert_eq!(events[0].max_tokens, Some(max));
    assert!(schedules.0.lock().unwrap().is_empty(), "no wait");
    assert_eq!(fixture.count("turn.completed").await, 1);
    fixture.assert_idle();
}

/// Below what a useful answer needs, the turn fails and a check for credits
/// is scheduled a minute out, without holding the worker.
#[tokio::test]
async fn a_balance_below_a_useful_answer_waits_for_credits() {
    let schedules = Arc::new(Schedules::default());
    let fixture = fixture(&schedules).await;
    fixture.refuse(refusal(283));
    let (worker, _) = fixture.worker("w1");
    let before = Utc::now();
    fixture.send("Review the pull request.").await;

    assert_eq!(run_until_idle(&worker).await, 1);

    assert_eq!(fixture.max_tokens(), vec![None], "no smaller retry");
    assert_eq!(
        fixture.credit_actions().await,
        vec![CreditShortfallAction::WaitStarted]
    );
    let started = &fixture.credit_events().await[0];
    assert_eq!(started.affordable_tokens, Some(283));
    let queued = schedules.queued();
    assert_eq!(queued.len(), 1);
    let next = queued[0].scheduled_at.unwrap();
    assert!(next >= before + chrono::Duration::seconds(PROBE_INTERVAL_SECS));
    assert_eq!(started.next_check_at, Some(next));
    let until = started.wait_until.unwrap();
    assert!(until >= before + chrono::Duration::seconds(WAIT_WINDOW_SECS - 1));
    // The person reads that the work continues by itself.
    let answers = fixture.answers().await;
    assert_eq!(answers.len(), 1);
    assert!(answers[0].contains("out of credits"), "{}", answers[0]);
    assert!(
        answers[0].contains("continues automatically"),
        "{}",
        answers[0]
    );
    assert_eq!(fixture.count("turn.failed").await, 1);
    fixture.assert_idle();
}

/// Checks read the balance without a model call. While credits are out the
/// check ends quietly and schedules the next one; once they are back the
/// work resumes, once.
#[tokio::test]
async fn a_check_that_sees_credits_back_resumes_the_work_once() {
    let schedules = Arc::new(Schedules::default());
    let fixture = fixture(&schedules).await;
    fixture.refuse(refusal(283));
    let (worker, _) = fixture.worker("w1");
    fixture.send("Review the pull request.").await;
    run_until_idle(&worker).await;
    fixture.balances(&[Some(CreditStatus::Exhausted), Some(CreditStatus::Available)]);

    // First check: still out.
    fixture.fire(&schedules.claim()).await;
    run_until_idle(&worker).await;
    assert_eq!(fixture.model_calls(), 1, "a check spends no tokens");
    assert_eq!(fixture.answers().await.len(), 1, "a check says nothing");
    assert_eq!(schedules.queued().len(), 1, "the next check");

    // Second check: back. The work resumes.
    fixture.fire(&schedules.claim()).await;
    run_until_idle(&worker).await;

    assert_eq!(fixture.model_calls(), 2);
    assert_eq!(
        fixture.answers().await.last().map(String::as_str),
        Some("Review submitted.")
    );
    assert_eq!(
        fixture.credit_actions().await,
        vec![
            CreditShortfallAction::WaitStarted,
            CreditShortfallAction::StillWaiting,
            CreditShortfallAction::Resumed,
        ]
    );
    let events = fixture.credit_events().await;
    assert_eq!(events[1].balance_checked, Some(true));
    assert_eq!(events[2].balance_checked, Some(true));
    assert_eq!(events[1].wait_until, events[0].wait_until, "one wait");
    assert!(schedules.queued().is_empty(), "nothing left to resume");
    assert_eq!(fixture.model.balance_reads.load(Ordering::SeqCst), 2);
    fixture.assert_idle();
}

/// A provider with no balance API is checked by the request itself, on the
/// same timer and in the same window.
#[tokio::test]
async fn without_a_balance_api_the_request_is_the_check() {
    let schedules = Arc::new(Schedules::default());
    let fixture = fixture(&schedules).await;
    fixture.refuse(refusal(283));
    fixture.refuse(refusal(283));
    let (worker, _) = fixture.worker("w1");
    fixture.send("Review the pull request.").await;
    run_until_idle(&worker).await;

    fixture.fire(&schedules.claim()).await;
    run_until_idle(&worker).await;
    assert_eq!(
        fixture.answers().await.len(),
        1,
        "a refused check says nothing"
    );
    fixture.fire(&schedules.claim()).await;
    run_until_idle(&worker).await;

    assert_eq!(fixture.model_calls(), 3);
    assert_eq!(
        fixture.answers().await.last().map(String::as_str),
        Some("Review submitted.")
    );
    let events = fixture.credit_events().await;
    let actions: Vec<_> = events.iter().map(|data| data.action).collect();
    assert_eq!(
        actions,
        vec![
            CreditShortfallAction::WaitStarted,
            CreditShortfallAction::StillWaiting,
            CreditShortfallAction::Resumed,
        ]
    );
    assert_eq!(events[1].balance_checked, Some(false));
    assert_eq!(events[2].balance_checked, Some(false));
    assert!(schedules.queued().is_empty());
    fixture.assert_idle();
}

/// The hour runs out: the last check stands down, and the turn fails with
/// the ordinary quota error. No new wait starts from it.
#[tokio::test]
async fn a_wait_that_runs_out_fails_with_the_quota_error() {
    let schedules = Arc::new(Schedules::default());
    let fixture = fixture(&schedules).await;
    fixture.balances(&[Some(CreditStatus::Exhausted)]);
    // The last check of a wait: the next one would land past its end.
    let deadline = Utc::now() + chrono::Duration::seconds(PROBE_INTERVAL_SECS / 2);
    let last = schedules
        .create_schedule(
            fixture.session_id,
            credit_shortfall::check_prompt(deadline),
            None,
            Some(Utc::now()),
            "UTC".to_string(),
        )
        .await
        .unwrap();
    let (worker, _) = fixture.worker("w1");
    fixture.fire(&last).await;
    schedules.claim();

    assert_eq!(run_until_idle(&worker).await, 1);

    assert_eq!(fixture.model_calls(), 0);
    assert_eq!(
        fixture.credit_actions().await,
        vec![CreditShortfallAction::GaveUp]
    );
    let answers = fixture.answers().await;
    assert_eq!(answers.len(), 1);
    assert_eq!(
        answers[0],
        "The AI provider account is out of credits or quota. Add credits or raise the provider account limits to continue."
    );
    assert!(schedules.queued().is_empty(), "no new wait");
    assert_eq!(fixture.count("turn.failed").await, 1);
    fixture.assert_idle();
}

/// A worker dies after a check scheduled the next one, before the check's
/// turn ended. The survivor finishes that turn without a second check, and
/// the wait resumes the work once.
#[tokio::test]
async fn a_worker_restart_during_the_wait_does_not_resume_twice() {
    let schedules = Arc::new(Schedules::default());
    let fixture = fixture(&schedules).await;
    fixture.refuse(refusal(283));
    let (worker, _) = fixture.worker("w1");
    fixture.send("Review the pull request.").await;
    run_until_idle(&worker).await;
    fixture.balances(&[
        Some(CreditStatus::Exhausted),
        Some(CreditStatus::Exhausted),
        Some(CreditStatus::Available),
        Some(CreditStatus::Available),
    ]);

    let (crashing, probe) = fixture.worker("w2");
    // The second flush follows the check's reason.
    probe.hang_at_flush.store(2, Ordering::SeqCst);
    fixture.fire(&schedules.claim()).await;
    crash_at(&fixture, &crashing, &probe.hung).await;
    assert_eq!(schedules.queued().len(), 1, "the check scheduled the next");

    let (survivor, _) = fixture.worker("w3");
    assert_eq!(run_until_idle(&survivor).await, 1);
    assert_eq!(schedules.queued().len(), 1, "no second check");

    fixture.fire(&schedules.claim()).await;
    run_until_idle(&survivor).await;

    assert_eq!(
        fixture
            .answers()
            .await
            .iter()
            .filter(|answer| *answer == "Review submitted.")
            .count(),
        1,
        "resumed once"
    );
    let resumed = fixture
        .credit_actions()
        .await
        .into_iter()
        .filter(|action| *action == CreditShortfallAction::Resumed)
        .count();
    assert_eq!(resumed, 1);
    assert!(schedules.queued().is_empty());
    fixture.assert_idle();
}
