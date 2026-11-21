//! A session asked to start from a copy of its source's sandbox gets one when
//! the provider can copy, and an ordinary fresh sandbox otherwise.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use async_trait::async_trait;
use everruns_contracts::sandbox_checkpoint::SandboxRef;
use everruns_contracts::typed_id::SessionId;
use everruns_core::tool_context::ToolContext;
use everruns_core::tools::ToolExecutionResult;
use serde_json::json;

use super::*;
use crate::sandbox_state::{SandboxStateError, SandboxStateStore, SandboxStateStoreExt};

const PROVIDER: &str = "fork-test-session-sandbox";

/// What the test provider was asked to do, keyed by the session it acted for.
#[derive(Default)]
struct Calls {
    created: Vec<SessionId>,
    forked: Vec<(SessionId, String)>,
    exec: Vec<(String, String)>,
}

static CALLS: LazyLock<Mutex<Calls>> = LazyLock::new(|| Mutex::new(Calls::default()));
/// Source sandboxes this provider refuses to copy.
static UNCOPYABLE: LazyLock<Mutex<Vec<String>>> = LazyLock::new(|| Mutex::new(Vec::new()));

fn instance(external_id: &str) -> SessionSandboxInstance {
    SessionSandboxInstance {
        external_id: external_id.to_string(),
        display_name: None,
        workspace_path: Some("/workspace".to_string()),
        provider_state: json!({}),
        metadata: json!({}),
    }
}

struct ForkTestProvider;

inventory::submit! {
    SessionSandboxProviderPlugin {
        factory: || Box::new(ForkTestProvider),
    }
}

#[async_trait]
impl SessionSandboxProvider for ForkTestProvider {
    fn id(&self) -> &str {
        PROVIDER
    }

    async fn create(
        &self,
        context: &dyn SessionSandboxContext,
        _config: &SessionSandboxConfig,
    ) -> Result<SessionSandboxInstance, ToolExecutionResult> {
        CALLS.lock().unwrap().created.push(context.session_id());
        Ok(instance(&format!("fresh-{}", context.session_id())))
    }

    async fn fork(
        &self,
        context: &dyn SessionSandboxContext,
        _config: &SessionSandboxConfig,
        source: &SessionSandboxInstance,
    ) -> Result<Option<SessionSandboxInstance>, ToolExecutionResult> {
        if UNCOPYABLE.lock().unwrap().contains(&source.external_id) {
            return Ok(None);
        }
        CALLS
            .lock()
            .unwrap()
            .forked
            .push((context.session_id(), source.external_id.clone()));
        Ok(Some(instance(&format!("copy-{}", context.session_id()))))
    }

    async fn resume(
        &self,
        _context: &dyn SessionSandboxContext,
        _config: &SessionSandboxConfig,
        instance: &SessionSandboxInstance,
    ) -> Result<SessionSandboxInstance, ToolExecutionResult> {
        Ok(instance.clone())
    }

    async fn pause(
        &self,
        _context: &dyn SessionSandboxContext,
        _config: &SessionSandboxConfig,
        instance: &SessionSandboxInstance,
    ) -> Result<SessionSandboxInstance, ToolExecutionResult> {
        Ok(instance.clone())
    }

    async fn delete(
        &self,
        _context: &dyn SessionSandboxContext,
        _config: &SessionSandboxConfig,
        _instance: &SessionSandboxInstance,
    ) -> Result<(), ToolExecutionResult> {
        Ok(())
    }

    async fn exec(
        &self,
        _context: &dyn SessionSandboxContext,
        _config: &SessionSandboxConfig,
        instance: &SessionSandboxInstance,
        request: &SessionSandboxExecRequest,
    ) -> Result<SessionSandboxExecResponse, ToolExecutionResult> {
        CALLS
            .lock()
            .unwrap()
            .exec
            .push((instance.external_id.clone(), request.command.clone()));
        Ok(SessionSandboxExecResponse {
            exit_code: 0,
            stdout: String::new(),
            stderr: String::new(),
            success: true,
            truncated: false,
            total_lines: 0,
            raw_output: None,
            hint: None,
        })
    }

    async fn read_file(
        &self,
        _context: &dyn SessionSandboxContext,
        _config: &SessionSandboxConfig,
        _instance: &SessionSandboxInstance,
        path: &str,
    ) -> Result<SessionSandboxReadFileResponse, ToolExecutionResult> {
        Err(ToolExecutionResult::tool_error(format!("no file {path}")))
    }

    async fn write_file(
        &self,
        _context: &dyn SessionSandboxContext,
        _config: &SessionSandboxConfig,
        _instance: &SessionSandboxInstance,
        path: &str,
        content: &[u8],
    ) -> Result<SessionSandboxWriteFileResponse, ToolExecutionResult> {
        Ok(SessionSandboxWriteFileResponse {
            path: path.to_string(),
            bytes_written: content.len(),
        })
    }

    async fn status(
        &self,
        _context: &dyn SessionSandboxContext,
        _config: &SessionSandboxConfig,
        state: &SessionSandboxState,
    ) -> Result<SessionSandboxStatusResponse, ToolExecutionResult> {
        Ok(SessionSandboxStatusResponse {
            provider: PROVIDER.to_string(),
            session_status: state.status,
            external_id: state.instance.external_id.clone(),
            display_name: None,
            workspace_path: None,
            metadata: json!({}),
        })
    }
}

/// States by session, plus pending copy requests that only a parent may make.
#[derive(Default)]
struct MemoryStore {
    states: Mutex<HashMap<SessionId, SessionSandboxState>>,
    parents: Mutex<HashMap<SessionId, SessionId>>,
    pending: Mutex<HashMap<SessionId, SessionId>>,
}

#[async_trait]
impl SandboxStateStore for MemoryStore {
    async fn load_current_state(
        &self,
        session_id: SessionId,
    ) -> Result<Option<SessionSandboxState>, SandboxStateError> {
        Ok(self.states.lock().unwrap().get(&session_id).cloned())
    }

    async fn load_state(
        &self,
        session_id: SessionId,
        provider: &str,
    ) -> Result<Option<SessionSandboxState>, SandboxStateError> {
        Ok(self
            .states
            .lock()
            .unwrap()
            .get(&session_id)
            .filter(|state| state.provider == provider)
            .cloned())
    }

    async fn save_state(
        &self,
        session_id: SessionId,
        state: &SessionSandboxState,
        _expected: Option<&SandboxRef>,
    ) -> Result<SandboxRef, SandboxStateError> {
        self.states
            .lock()
            .unwrap()
            .insert(session_id, state.clone());
        Ok(SandboxRef {
            id: uuid::Uuid::new_v4(),
            generation: 1,
        })
    }

    async fn delete_state(
        &self,
        session_id: SessionId,
        _provider: &str,
        _expected: Option<&SandboxRef>,
    ) -> Result<bool, SandboxStateError> {
        Ok(self.states.lock().unwrap().remove(&session_id).is_some())
    }

    async fn request_fork(
        &self,
        session_id: SessionId,
        source_session_id: SessionId,
    ) -> Result<bool, SandboxStateError> {
        if self.parents.lock().unwrap().get(&session_id) != Some(&source_session_id) {
            return Ok(false);
        }
        self.pending
            .lock()
            .unwrap()
            .insert(session_id, source_session_id);
        Ok(true)
    }

    async fn take_fork_source(
        &self,
        session_id: SessionId,
    ) -> Result<Option<SessionId>, SandboxStateError> {
        Ok(self.pending.lock().unwrap().remove(&session_id))
    }
}

fn config() -> SessionSandboxConfig {
    serde_json::from_value(json!({
        "provider": PROVIDER,
        "init": { "commands": ["install-deps"] },
    }))
    .unwrap()
}

fn context(store: &Arc<MemoryStore>, session_id: SessionId) -> ToolContext {
    let store: Arc<dyn SandboxStateStore> = store.clone();
    ToolContext::new(session_id).with_extension(Arc::new(SandboxStateStoreExt(store)))
}

/// A parent with a running, initialized sandbox and a child linked to it.
async fn parent_and_child(store: &Arc<MemoryStore>) -> (SessionId, SessionId) {
    let parent = SessionId::new();
    let child = SessionId::new();
    store.parents.lock().unwrap().insert(child, parent);
    ensure_session_sandbox_running(&context(store, parent), &config())
        .await
        .unwrap();
    (parent, child)
}

#[tokio::test]
async fn a_requested_copy_starts_from_the_parents_sandbox() {
    let store = Arc::new(MemoryStore::default());
    let (parent, child) = parent_and_child(&store).await;

    assert!(
        crate::sandbox_state::request_session_sandbox_fork(&context(&store, parent), child)
            .await
            .unwrap()
    );
    let state = ensure_session_sandbox_running(&context(&store, child), &config())
        .await
        .unwrap();

    assert_eq!(state.instance.external_id, format!("copy-{child}"));
    assert_eq!(
        state.instance.metadata["forked_from"],
        json!({"session_id": parent.to_string(), "external_id": format!("fresh-{parent}")})
    );
    // The copy is already initialized and holds the files: no init, no seed.
    assert!(state.init_completed_at.is_some());
    assert!(!workspace_seed::is_pending(&state.instance));
    let calls = CALLS.lock().unwrap();
    assert!(!calls.created.contains(&child));
    assert!(
        !calls
            .exec
            .iter()
            .any(|(external_id, _)| external_id == &format!("copy-{child}"))
    );
}

#[tokio::test]
async fn without_a_request_the_child_starts_fresh() {
    let store = Arc::new(MemoryStore::default());
    let (_parent, child) = parent_and_child(&store).await;

    let state = ensure_session_sandbox_running(&context(&store, child), &config())
        .await
        .unwrap();

    assert_eq!(state.instance.external_id, format!("fresh-{child}"));
    assert!(CALLS.lock().unwrap().created.contains(&child));
}

#[tokio::test]
async fn a_sandbox_the_provider_cannot_copy_falls_back_to_fresh() {
    let store = Arc::new(MemoryStore::default());
    let (parent, child) = parent_and_child(&store).await;
    UNCOPYABLE.lock().unwrap().push(format!("fresh-{parent}"));

    crate::sandbox_state::request_session_sandbox_fork(&context(&store, parent), child)
        .await
        .unwrap();
    let state = ensure_session_sandbox_running(&context(&store, child), &config())
        .await
        .unwrap();

    assert_eq!(state.instance.external_id, format!("fresh-{child}"));
    assert!(state.instance.metadata.get("forked_from").is_none());
    // The fresh sandbox is initialized as usual.
    assert!(
        CALLS
            .lock()
            .unwrap()
            .exec
            .contains(&(format!("fresh-{child}"), "install-deps".to_string()))
    );
}

#[tokio::test]
async fn only_the_source_session_can_ask_for_a_copy() {
    let store = Arc::new(MemoryStore::default());
    let (_parent, child) = parent_and_child(&store).await;
    let stranger = SessionId::new();

    assert!(
        !crate::sandbox_state::request_session_sandbox_fork(&context(&store, stranger), child)
            .await
            .unwrap()
    );
    let state = ensure_session_sandbox_running(&context(&store, child), &config())
        .await
        .unwrap();
    assert_eq!(state.instance.external_id, format!("fresh-{child}"));
}

#[tokio::test]
async fn a_host_without_a_state_store_cannot_record_a_copy() {
    let parent = ToolContext::new(SessionId::new());
    assert!(
        !crate::sandbox_state::request_session_sandbox_fork(&parent, SessionId::new())
            .await
            .unwrap()
    );
}
