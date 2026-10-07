use super::*;
use async_trait::async_trait;
use everruns_contracts::runtime::tool_execution::BudgetChecker;
use everruns_contracts::runtime::{
    EgressResponse, EgressResult, EgressService, EgressStreamResponse, Event, EventEmitter,
};
use std::sync::{Arc, Mutex};

struct BudgetFixture(&'static str);
#[async_trait]
impl BudgetChecker for BudgetFixture {
    async fn check_budgets(
        &self,
        _: &str,
    ) -> everruns_contracts::error::Result<everruns_contracts::runtime::budget::BudgetToolResponse>
    {
        Ok(everruns_contracts::runtime::budget::BudgetToolResponse {
            status: self.0.into(),
            budgets: vec![],
            hint: None,
        })
    }
}
#[derive(Default)]
struct Events(Mutex<Vec<Value>>);
#[async_trait]
impl EventEmitter for Events {
    async fn emit(&self, request: EventRequest) -> everruns_contracts::error::Result<Event> {
        self.0
            .lock()
            .unwrap()
            .push(serde_json::to_value(&request).unwrap());
        Ok(request.into_event(everruns_contracts::typed_id::EventId::new(), 1))
    }
}
struct Network(Vec<u8>);
#[async_trait]
impl EgressService for Network {
    async fn send(&self, _: EgressRequest) -> EgressResult<EgressResponse> {
        panic!("must stream")
    }
    async fn send_stream(&self, request: EgressRequest) -> EgressResult<EgressStreamResponse> {
        assert_eq!(request.kind, EgressRequestKind::Provider);
        assert_eq!(request.timeout_ms, Some(10_000));
        assert_eq!(request.headers["authorization"], "Bearer saved-account-key");
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(body["model"], "typesafe/jev-1.13");
        Ok(EgressStreamResponse {
            status: 200,
            headers: Default::default(),
            body: Box::pin(futures::stream::iter(vec![Ok(self.0.clone())])),
        })
    }
}
fn binding() -> DecisionModelBinding {
    DecisionModelBinding {
        model_id: "saved-model".into(),
        provider_id: "saved-account".into(),
        provider_type: "openrouter".into(),
        model: "jev-1.13.0".into(),
        profile_key: "typesafe/jev-1.13.0".into(),
        api_key: "saved-account-key".into(),
        base_url: None,
        headers: [("AUTHORIZATION".into(), "stale".into())].into(),
    }
}
fn context(response: Vec<u8>, status: &'static str) -> (ToolContext, Arc<Events>) {
    let events = Arc::new(Events::default());
    let mut context = ToolContext::new(everruns_contracts::typed_id::SessionId::new());
    context.budget_checker = Some(Arc::new(BudgetFixture(status)));
    context.egress_service = Some(Arc::new(Network(response)));
    context.event_emitter = Some(events.clone());
    context.event_context = Some(everruns_contracts::runtime::EventContext::empty());
    (context, events)
}
fn input() -> EvaluateInput {
    serde_json::from_value(json!({"state":"private content", "questions":[{"id":"q","type":"noul","instructions":"True?"}]})).unwrap()
}
#[tokio::test]
async fn exact_account_snapshot_and_usage_are_recorded_without_state() {
    let response = json!({"id":"receipt", "model":"typesafe/jev-1.13-20260917","provider":"TypeSafe","answers":{"q":{"type":"noul","noul":0.97}},"usage":{"input_tokens":100,"output_tokens":2,"cost":0.0000042}});
    let (context, events) = context(serde_json::to_vec(&response).unwrap(), "active");
    let result = evaluate(binding(), input(), &context).await.unwrap();
    assert_eq!(result["model"], response["model"]);
    let events = events.0.lock().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["data"]["metadata"]["provider"], "saved-account");
    assert_eq!(
        events[0]["data"]["metadata"]["response_model"],
        response["model"]
    );
    assert_eq!(
        events[0]["data"]["metadata"]["usage"]["actual_cost_usd"],
        0.0000042
    );
    assert!(!events[0].to_string().contains("private content"));
    assert!(!events[0].to_string().contains("saved-account-key"));
}
#[tokio::test]
async fn charged_invalid_answers_are_metered_before_rejection() {
    let response =
        json!({"answers":{},"usage":{"input_tokens":100,"output_tokens":2,"cost":0.001}});
    let (context, events) = context(serde_json::to_vec(&response).unwrap(), "active");
    assert!(evaluate(binding(), input(), &context).await.is_err());
    let events = events.0.lock().unwrap();
    assert_eq!(events[0]["data"]["metadata"]["success"], false);
    assert_eq!(
        events[0]["data"]["metadata"]["usage"]["actual_cost_usd"],
        0.001
    );
}
#[tokio::test]
async fn exhausted_budget_stops_before_network_and_oversized_response_fails_closed() {
    let (blocked, events) = context(vec![], "exhausted");
    assert_eq!(
        evaluate(binding(), input(), &blocked).await.unwrap_err(),
        "Decision budget exhausted"
    );
    assert!(events.0.lock().unwrap().is_empty());
    let (oversized, events) = context(vec![b' '; 2 * 1024 * 1024 + 1], "active");
    assert!(
        evaluate(binding(), input(), &oversized)
            .await
            .unwrap_err()
            .contains("size limit")
    );
    assert_eq!(events.0.lock().unwrap().len(), 1);
}

/// Answers like OpenAI's Decisions API and checks the request it got.
struct OpenAiNetwork;
#[async_trait]
impl EgressService for OpenAiNetwork {
    async fn send(&self, _: EgressRequest) -> EgressResult<EgressResponse> {
        panic!("must stream")
    }
    async fn send_stream(&self, request: EgressRequest) -> EgressResult<EgressStreamResponse> {
        assert_eq!(request.kind, EgressRequestKind::Provider);
        assert_eq!(request.url, "https://api.openai.com/v1/decisions");
        assert_eq!(request.headers["authorization"], "Bearer saved-account-key");
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(body["model"], "gpt-6-luna");
        assert_eq!(body["questions"][0]["type"], "predicate");
        assert_eq!(body["questions"][0]["name"], "q0");
        let response = json!({"model":"gpt-6-luna","answers":[{"type":"predicate","name":"q0","probability":0.97}],"usage":{"input_tokens":1000,"output_tokens":0}});
        Ok(EgressStreamResponse {
            status: 200,
            headers: Default::default(),
            body: Box::pin(futures::stream::iter(vec![Ok(serde_json::to_vec(
                &response,
            )
            .unwrap())])),
        })
    }
}

#[tokio::test]
async fn an_openai_decision_model_runs_on_the_decisions_api_and_is_priced() {
    let (mut context, events) = context(vec![], "active");
    context.egress_service = Some(Arc::new(OpenAiNetwork));
    let binding = DecisionModelBinding {
        provider_type: "openai".into(),
        model: "gpt-6-luna-decisions".into(),
        profile_key: "openai/gpt-6-luna-decisions".into(),
        headers: Default::default(),
        ..binding()
    };
    let result = evaluate(binding, input(), &context).await.unwrap();
    assert_eq!(result["model"], "gpt-6-luna");
    assert_eq!(result["answers"]["q"]["probability_yes"], 0.97);
    let events = events.0.lock().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["data"]["metadata"]["success"], true);
    // $0.10 per 1M input tokens, output free.
    let cost = events[0]["data"]["metadata"]["usage"]["estimated_cost_usd"]
        .as_f64()
        .unwrap();
    assert!((cost - 0.0001).abs() < 1e-12, "{cost}");
}

/// A deployment-owned check asked of the org's model: the binding's model
/// wins over whatever the caller named, and the call is budgeted and metered
/// like the Jev tool.
#[tokio::test]
async fn host_checks_run_the_org_model_on_the_jev_path() {
    let response = json!({"model":"typesafe/jev-1.13","answers":{"q":{"type":"noul","noul":0.2}},"usage":{"input_tokens":10,"output_tokens":1}});
    let (active, events) = context(serde_json::to_vec(&response).unwrap(), "active");
    let request = DecisionRequest::new("guarded content")
        .ask("q", DecisionQuestion::noul("Violates the policy?"))
        .model("caller-named-model");
    let outcome = BoundDecisionExecutor
        .evaluate(binding(), request.clone(), &active)
        .await
        .unwrap();
    assert_eq!(
        outcome.get("q").and_then(|a| a.probability_yes()),
        Some(0.2)
    );
    assert_eq!(events.0.lock().unwrap().len(), 1);
    let (blocked, _) = context(vec![], "exhausted");
    assert!(
        BoundDecisionExecutor
            .evaluate(binding(), request, &blocked)
            .await
            .is_err()
    );
}

/// Before a session exists there is nothing to budget or bill, so the call
/// needs only egress.
#[tokio::test]
async fn unmetered_decisions_need_only_egress() {
    let response = json!({"model":"typesafe/jev-1.13","answers":{"q":{"type":"noul","noul":0.95}}});
    let request = DecisionRequest::new("a Slack message")
        .ask("q", DecisionQuestion::noul("Should the agent answer?"));
    let outcome = evaluate_unmetered(
        &binding(),
        request,
        &Network(serde_json::to_vec(&response).unwrap()),
    )
    .await
    .unwrap();
    assert_eq!(
        outcome.get("q").and_then(|a| a.probability_yes()),
        Some(0.95)
    );
}
