//! Regression coverage for the host's injected shell-hook enforcement boundary.

use async_trait::async_trait;
use everruns_core::hook_executor::{BashExecOutput, BashHookDispatcher, ExecutorOpts, HookPayload};
use everruns_core::host::{HostBackends, InProcessRuntime, InProcessRuntimeBuilder};
use everruns_core::session_files::SessionFileSystem;
use everruns_core::user_hook_types::{
    ExecutorSpec, HookEvent, HookMatcher, HookSource, OnError, UserHookSpec,
};
use everruns_core::{Capability, CapabilityStatus};
use everruns_llmsim::{LlmSimConfig, LlmSimRuntimeExt};
use std::collections::BTreeMap;
use std::sync::Arc;

struct PromptHook(&'static str);

impl Capability for PromptHook {
    fn id(&self) -> &str {
        "injected_prompt_hook"
    }
    fn name(&self) -> &str {
        "Injected prompt hook"
    }
    fn description(&self) -> &str {
        "Exercises shell-hook enforcement"
    }
    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }
    fn user_hooks(&self) -> Vec<UserHookSpec> {
        vec![UserHookSpec {
            id: Some("enforcement".into()),
            event: HookEvent::UserPromptSubmit,
            matcher: HookMatcher::default(),
            executor: ExecutorSpec::Bash {
                command: self.0.into(),
                env: BTreeMap::new(),
            },
            timeout_ms: 5000,
            on_error: OnError::Block,
            description: None,
            source: HookSource::UserConfig,
        }]
    }
}

struct RecordingDispatcher(Arc<dyn SessionFileSystem>);

#[async_trait]
impl BashHookDispatcher for RecordingDispatcher {
    async fn dispatch(
        &self,
        payload: &HookPayload,
        _command: &str,
        _extra_env: &BTreeMap<String, String>,
        _opts: &ExecutorOpts,
    ) -> Result<BashExecOutput, String> {
        self.0
            .write_file(
                payload.session_id,
                "/injected-hook.txt",
                "caller-selected",
                "text",
            )
            .await
            .map_err(|error| error.to_string())?;
        Ok(BashExecOutput {
            exit_code: 0,
            stdout: "{\"decision\":\"allow\"}".into(),
            stderr: String::new(),
        })
    }
}

async fn guarded_runtime(backends: HostBackends, command: &'static str) -> InProcessRuntime {
    InProcessRuntimeBuilder::new()
        .backends(backends)
        .capability(PromptHook(command))
        .llm_sim_as_default(LlmSimConfig::fixed("reason was reached"))
        .single_session(|session| {
            session
                .harness("guarded", "Guard prompts.")
                .harness_capability("injected_prompt_hook")
                .agent("agent", "Reply once.")
        })
        .build()
        .await
        .expect("guarded runtime builds")
}

const BLOCK: &str = r#"echo '{"decision":"block","reason":"policy","user_message":"blocked"}'"#;

#[tokio::test]
async fn core_without_a_dispatcher_fails_closed_for_block_on_error_hooks() {
    let runtime = guarded_runtime(HostBackends::in_memory(), "true").await;
    let id = runtime.default_session_id().unwrap();
    let result = runtime.run_text_turn(id, "hello").await.unwrap();
    assert!(!result.success);
    assert_eq!(result.error.as_deref(), Some("blocked_by_user_prompt_hook"));
    assert!(!result.response.contains("reason was reached"));
}

async fn assert_custom_factory(facade_batteries: bool) {
    let backends = HostBackends::in_memory()
        .with_bash_hook_dispatcher_factory(Arc::new(|files| Arc::new(RecordingDispatcher(files))));
    let backends = if facade_batteries {
        everruns::batteries::runtime_backends(backends)
    } else {
        backends
    };
    let runtime = guarded_runtime(backends, BLOCK).await;
    let id = runtime.default_session_id().unwrap();
    let result = runtime.run_text_turn(id, "hello").await.unwrap();
    assert!(
        result.success,
        "caller-selected dispatcher must retain authority"
    );
    assert_eq!(result.response, "reason was reached");
    let file = runtime
        .read_file(id, "/injected-hook.txt")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(file.content.as_deref(), Some("caller-selected"));
}

#[tokio::test]
async fn core_dispatcher_factory_receives_the_runtime_session_filesystem() {
    assert_custom_factory(false).await;
}

#[cfg(feature = "bashkit")]
#[tokio::test]
async fn facade_batteries_preserve_a_caller_selected_dispatcher() {
    assert_custom_factory(true).await;
}

#[cfg(feature = "bashkit")]
#[tokio::test]
async fn facade_injects_a_real_bashkit_dispatcher_by_default() {
    let runtime = guarded_runtime(
        everruns::batteries::runtime_backends(HostBackends::in_memory()),
        r#"echo bashkit > /workspace/default-hook.txt; echo '{"decision":"block","reason":"policy","user_message":"blocked"}'"#,
    )
    .await;
    let id = runtime.default_session_id().unwrap();
    let result = runtime.run_text_turn(id, "hello").await.unwrap();
    assert!(!result.success);
    let marker = runtime
        .read_file(id, "/default-hook.txt")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(marker.content.as_deref().map(str::trim), Some("bashkit"));
    assert_eq!(result.error.as_deref(), Some("blocked_by_user_prompt_hook"));
}
