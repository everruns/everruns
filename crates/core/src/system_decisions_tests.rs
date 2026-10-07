use super::*;
use everruns_contracts::runtime::connection_services::{
    DecisionModelBinding, DecisionModelExecutor, ProviderCredentialStore, ProviderCredentials,
};
use everruns_contracts::typed_id::SessionId;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Counts how often the deployment is asked.
#[derive(Default)]
struct Deployment(AtomicUsize);

#[async_trait]
impl DecisionsService for Deployment {
    fn is_configured(&self) -> bool {
        true
    }

    async fn evaluate(&self, _request: DecisionRequest) -> Result<DecisionOutcome> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(outcome("deployment"))
    }
}

/// Records the binding it was asked to run.
#[derive(Default)]
struct Executor(std::sync::Mutex<Vec<String>>);

#[async_trait]
impl DecisionModelExecutor for Executor {
    async fn evaluate(
        &self,
        binding: DecisionModelBinding,
        _request: DecisionRequest,
        context: &ToolContext,
    ) -> Result<DecisionOutcome> {
        assert!(
            context.event_context.is_some(),
            "usage needs an event context"
        );
        self.0.lock().unwrap().push(binding.model_id);
        Ok(outcome("organization"))
    }
}

/// An org choice: `None` is an org that opted in without a usable model.
struct Store(Option<SystemDecisionModel>);

#[async_trait]
impl ProviderCredentialStore for Store {
    async fn get_system_decision_model(&self, _: SessionId) -> Result<SystemDecisionModel> {
        self.0
            .clone()
            .ok_or_else(|| AgentLoopError::store("Organization decision model is unavailable"))
    }

    async fn get_default_provider_credentials(
        &self,
        _: &str,
    ) -> Result<Option<ProviderCredentials>> {
        Ok(None)
    }
}

fn outcome(model: &str) -> DecisionOutcome {
    DecisionOutcome {
        model: model.into(),
        ..Default::default()
    }
}

fn binding() -> DecisionModelBinding {
    DecisionModelBinding {
        model_id: "model_org".into(),
        provider_id: "provider_org".into(),
        provider_type: "typesafe".into(),
        model: "jev-1.13.0".into(),
        profile_key: "typesafe/jev-1.13.0".into(),
        api_key: "org-key".into(),
        base_url: None,
        headers: Default::default(),
    }
}

fn context(
    store: Option<Store>,
    deployment: &Arc<Deployment>,
    executor: Option<&Arc<Executor>>,
) -> ToolContext {
    let mut context = ToolContext::new(SessionId::new());
    context.decisions = Some(deployment.clone());
    context.provider_credential_store = store.map(|s| Arc::new(s) as Arc<_>);
    context.event_context = Some(EventContext::empty());
    if let Some(executor) = executor {
        context
            .extensions
            .insert(Arc::new(DecisionModelExecutorExt(executor.clone())));
    }
    context
}

#[tokio::test]
async fn a_host_without_org_settings_keeps_the_deployment() {
    let deployment = Arc::new(Deployment::default());
    let service = for_context(&context(None, &deployment, None)).unwrap();
    service.evaluate(DecisionRequest::new("x")).await.unwrap();
    assert_eq!(deployment.0.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn the_deployment_answers_when_the_org_leaves_it_the_choice() {
    let deployment = Arc::new(Deployment::default());
    let executor = Arc::new(Executor::default());
    let ctx = context(
        Some(Store(Some(SystemDecisionModel::Deployment))),
        &deployment,
        Some(&executor),
    );
    let outcome = for_context(&ctx)
        .unwrap()
        .evaluate(DecisionRequest::new("x"))
        .await
        .unwrap();
    assert_eq!(outcome.model, "deployment");
    assert!(executor.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn an_org_that_opted_in_is_answered_by_its_own_model() {
    let deployment = Arc::new(Deployment::default());
    let executor = Arc::new(Executor::default());
    let ctx = context(
        Some(Store(Some(SystemDecisionModel::Organization(binding())))),
        &deployment,
        Some(&executor),
    );
    let outcome = for_context(&ctx)
        .unwrap()
        .evaluate(DecisionRequest::new("x"))
        .await
        .unwrap();
    assert_eq!(outcome.model, "organization");
    assert_eq!(*executor.0.lock().unwrap(), ["model_org"]);
    assert_eq!(deployment.0.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn an_unusable_org_model_errors_instead_of_spending_deployment_keys() {
    let deployment = Arc::new(Deployment::default());
    let executor = Arc::new(Executor::default());
    let missing = context(Some(Store(None)), &deployment, Some(&executor));
    assert!(
        for_context(&missing)
            .unwrap()
            .evaluate(DecisionRequest::new("x"))
            .await
            .is_err()
    );
    // A host that cannot run org models refuses too.
    let no_executor = context(
        Some(Store(Some(SystemDecisionModel::Organization(binding())))),
        &deployment,
        None,
    );
    assert!(
        for_context(&no_executor)
            .unwrap()
            .evaluate(DecisionRequest::new("x"))
            .await
            .is_err()
    );
    assert_eq!(deployment.0.load(Ordering::SeqCst), 0);
    assert!(executor.0.lock().unwrap().is_empty());
}

#[test]
fn output_checks_carry_the_turn_for_usage() {
    let execution = ExecutionContext::new(
        SessionId::new(),
        everruns_contracts::typed_id::TurnId::new(),
        everruns_contracts::typed_id::MessageId::new(),
    );
    let services = ToolContextServices::default();
    // No credential store and no deployment: nothing to ask.
    assert!(for_turn(&services, &execution).is_none());
}
