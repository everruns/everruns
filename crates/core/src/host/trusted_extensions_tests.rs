use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use everruns_contracts::CapabilityRef;
use everruns_contracts::error::Result;
use everruns_contracts::knowledge::{KnowledgeQuery, KnowledgeResults};
use everruns_contracts::runtime::budget::BudgetToolResponse;
use everruns_contracts::tool_types::ToolCall;
use everruns_contracts::typed_id::{AgentId, MessageId, SessionId};

use super::*;
use crate::Capability;
use crate::host::{InProcessRuntime, InProcessRuntimeBuilder, RuntimeHostAdapter};

struct Spending(&'static str);
#[async_trait]
impl BudgetChecker for Spending {
    async fn check_budgets(&self, _session_id: &str) -> Result<BudgetToolResponse> {
        Ok(BudgetToolResponse {
            status: self.0.into(),
            budgets: Vec::new(),
            hint: None,
        })
    }
}

struct Knowledge;
#[async_trait]
impl KnowledgeRuntime for Knowledge {
    async fn search(&self, _: SessionId, _: KnowledgeQuery) -> Result<KnowledgeResults> {
        unreachable!("automatic context only")
    }
    async fn automatic_context(
        &self,
        _: SessionId,
        _: MessageId,
        query: String,
    ) -> Result<Option<String>> {
        Ok(Some(format!("evidence for {query}")))
    }
}

struct Stub;
impl crate::Capability for Stub {
    fn id(&self) -> &str {
        crate::host::SESSION_CAPABILITY_ID
    }
    fn name(&self) -> &str {
        "Platform session"
    }
    fn description(&self) -> &str {
        "the platform's own session capability"
    }
}

struct Unknown;
impl crate::Capability for Unknown {
    fn id(&self) -> &str {
        "no_such_builtin"
    }
    fn name(&self) -> &str {
        "Unknown"
    }
    fn description(&self) -> &str {
        "replaces nothing"
    }
}

fn builder() -> InProcessRuntimeBuilder {
    InProcessRuntimeBuilder::new().model_spec(everruns_contracts::model_spec::ModelSpec::on(
        "test", "model",
    ))
}

async fn runtime(extensions: HostExtensions) -> Result<InProcessRuntime> {
    builder()
        .backends(crate::host::HostBackends::in_memory().with_extensions(extensions))
        .build()
        .await
}

#[tokio::test]
async fn an_empty_bundle_leaves_the_runtime_as_it_was() {
    let runtime = runtime(HostExtensions::default()).await.unwrap();
    assert!(runtime.budget_checker(1, None).is_none());
    assert!(runtime.payment_authority(1, None).is_none());
    assert!(runtime.leased_resource_store(1).is_none());
    assert!(runtime.durable_tool_result_store().is_none());
    assert!(
        runtime
            .hosted_mcp_resolver(1, SessionId::new(), None, MessageId::new())
            .is_none()
    );
    assert!(
        runtime.egress_service().is_some(),
        "the composition's egress"
    );
}

#[tokio::test]
async fn each_slot_reaches_the_turn_through_its_adapter_method() {
    let egress: Arc<dyn EgressService> = Arc::new(crate::DisabledEgressService);
    let asked = Arc::new(Mutex::new(Vec::new()));
    let seen = asked.clone();
    let mut extensions = HostExtensions::default();
    extensions.composition.egress = Some(egress.clone());
    extensions.authorities.budget = Some(Arc::new(move |agent: Option<AgentId>| {
        seen.lock().unwrap().push(agent);
        Some(Arc::new(Spending("exhausted")) as Arc<dyn BudgetChecker>)
    }));
    extensions.hooks.knowledge = Some(Arc::new(Knowledge));
    let runtime = runtime(extensions).await.unwrap();

    let agent = AgentId::new();
    let budgets = runtime
        .budget_checker(1, Some(agent))
        .unwrap()
        .check_budgets("s")
        .await
        .unwrap();
    assert_eq!(budgets.status, "exhausted");
    assert_eq!(*asked.lock().unwrap(), [Some(agent)], "chosen per agent");
    assert!(Arc::ptr_eq(&runtime.egress_service().unwrap(), &egress));
    assert!(runtime.host_extensions().hooks.knowledge.is_some());
}

#[tokio::test]
async fn a_declared_replacement_takes_the_builtin_id_and_an_unknown_one_fails() {
    let mut extensions = HostExtensions::default();
    extensions.composition.replaced_capabilities = vec![Arc::new(Stub)];
    let runtime = runtime(extensions).await.unwrap();
    let registry = runtime.capability_registry();
    let session = registry.get(crate::host::SESSION_CAPABILITY_ID).unwrap();
    assert_eq!(session.description(), Stub.description());

    let mut extensions = HostExtensions::default();
    extensions.composition.replaced_capabilities = vec![Arc::new(Unknown)];
    let error = runtime_error(extensions).await;
    assert!(error.contains("no_such_builtin"), "{error}");
}

async fn runtime_error(extensions: HostExtensions) -> String {
    match runtime(extensions).await {
        Ok(_) => panic!("a replacement of an unknown capability must fail the build"),
        Err(error) => error.to_string(),
    }
}

#[tokio::test]
async fn the_host_wait_predicate_adds_to_the_runtimes_own() {
    let mut extensions = HostExtensions::default();
    extensions.hooks.waits_on_person =
        Some(Arc::new(|call: &ToolCall| call.name == "approve_payment"));
    let runtime = builder()
        .backends(crate::host::HostBackends::in_memory().with_extensions(extensions))
        .waits_on_person(|call: &ToolCall| call.name == "ask_reviewer")
        .build()
        .await
        .unwrap();
    let waits = runtime.waits_on_person_predicate().unwrap();
    let call = |name: &str| ToolCall {
        id: "c".into(),
        name: name.into(),
        arguments: serde_json::json!({}),
    };
    assert!(waits(&call("approve_payment")));
    assert!(waits(&call("ask_reviewer")));
    assert!(!waits(&call("write_file")));
}

#[tokio::test]
async fn automatic_knowledge_adds_evidence_only_when_configured() {
    let session = SessionId::new();
    let input = MessageId::new();
    let automatic = [CapabilityRef::with_config(
        "knowledge",
        serde_json::json!({"automatic": true}),
    )];
    let mut messages = vec![crate::RuntimeMessage::user("what is a cell?")];
    add_automatic_knowledge(&Knowledge, &automatic, session, input, &mut messages).await;
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[1].text(), Some("evidence for what is a cell?"));

    let manual = [CapabilityRef::new("knowledge")];
    let mut messages = vec![crate::RuntimeMessage::user("what is a cell?")];
    add_automatic_knowledge(&Knowledge, &manual, session, input, &mut messages).await;
    assert_eq!(messages.len(), 1);
}
