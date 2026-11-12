//! Lets a host cancel one running tool call without cancelling its turn.
//!
//! Decision: cancelling a turn drops the whole act, so every call in it stops.
//! An editor that shows each call (ACP `tool_call/cancel`) needs to stop one
//! long command and let the model see that it was stopped and carry on. The
//! act races each inline call against a token the host can fire by
//! `(session, tool call id)`; when it fires, the call's future is dropped (a
//! tool's child process dies with it, as on a turn cancel), the call's own
//! [`ToolContext::cancellation`](crate::ToolContext) fires for work it left
//! running, and the call completes with status [`CANCELLED_STATUS`] and an
//! error the model reads.
//!
//! Decision: every inline call registers, in the [`ToolCallCancels`] a host
//! put in the tool context extensions, else in the process-wide [`shared`]
//! one. So an in-process embedder cancels through its session
//! (`everruns::Session::cancel_tool_call`) without wiring anything, and a host
//! that wants its own registry still can. Only calls the act runs inline register: a detached call
//! ([`super::tool_detach`]) belongs to its host once it detaches, and a call a
//! pre-tool hook blocked or deferred never runs.
//!
//! The registry is in-process. A host whose tool calls run on another worker
//! routes the cancel there itself.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use everruns_contracts::typed_id::SessionId;
use tokio_util::sync::CancellationToken;

use crate::engine::tool_types::ToolResult;

/// The `tool.completed` status of a call a host cancelled.
pub const CANCELLED_STATUS: &str = "cancelled";

/// What the model reads for a call that was cancelled.
pub const CANCELLED_MESSAGE: &str =
    "The user cancelled this tool call before it finished. Its effects may be partial.";

type Key = (SessionId, String);

/// The running inline tool calls a host may cancel, by session and call id.
#[derive(Default)]
pub struct ToolCallCancels {
    calls: Mutex<HashMap<Key, (u64, CancellationToken)>>,
    next: AtomicU64,
}

impl std::fmt::Debug for ToolCallCancels {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolCallCancels").finish_non_exhaustive()
    }
}

impl ToolCallCancels {
    pub fn new() -> Self {
        Self::default()
    }

    /// Cancel a running call. `false` when no such call is running (it is
    /// unknown, already finished, or not cancellable).
    pub fn cancel(&self, session_id: SessionId, tool_call_id: &str) -> bool {
        let calls = self.calls.lock().unwrap_or_else(|e| e.into_inner());
        match calls.get(&(session_id, tool_call_id.to_string())) {
            Some((_, token)) if !token.is_cancelled() => {
                token.cancel();
                true
            }
            _ => false,
        }
    }

    /// Whether a call is running and can still be cancelled.
    pub fn is_running(&self, session_id: SessionId, tool_call_id: &str) -> bool {
        let calls = self.calls.lock().unwrap_or_else(|e| e.into_inner());
        calls
            .get(&(session_id, tool_call_id.to_string()))
            .is_some_and(|(_, token)| !token.is_cancelled())
    }

    /// Track a call until the returned registration drops.
    pub fn register(self: &Arc<Self>, session_id: SessionId, tool_call_id: &str) -> Registration {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let token = CancellationToken::new();
        let key = (session_id, tool_call_id.to_string());
        self.calls
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key.clone(), (id, token.clone()));
        Registration {
            cancels: self.clone(),
            key,
            id,
            token,
        }
    }
}

/// The registry calls use when the host installed none.
pub fn shared() -> Arc<ToolCallCancels> {
    static SHARED: std::sync::OnceLock<Arc<ToolCallCancels>> = std::sync::OnceLock::new();
    SHARED
        .get_or_init(|| Arc::new(ToolCallCancels::new()))
        .clone()
}

/// A call tracked by [`ToolCallCancels`]; dropping it forgets the call.
pub struct Registration {
    cancels: Arc<ToolCallCancels>,
    key: Key,
    id: u64,
    token: CancellationToken,
}

impl Registration {
    /// Fires when the host cancels the call.
    pub fn token(&self) -> &CancellationToken {
        &self.token
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        let mut calls = self.cancels.calls.lock().unwrap_or_else(|e| e.into_inner());
        // A retried call may have registered the same id since; leave it.
        if calls.get(&self.key).is_some_and(|(id, _)| *id == self.id) {
            calls.remove(&self.key);
        }
    }
}

/// Run an inline call until it settles or a host cancels it. Registers the
/// call in the context's [`ToolCallCancels`], else in [`shared`], for as long
/// as it runs. On cancel the call's future is dropped and its own
/// cancellation token fires, so work it left running stops now rather than
/// after the post-tool hooks. The `bool` is whether it was cancelled.
pub(super) async fn race(
    context: &crate::ToolContext,
    session_id: SessionId,
    invoke: impl std::future::Future<Output = (crate::engine::error::Result<ToolResult>, Option<u64>)>,
) -> (crate::engine::error::Result<ToolResult>, Option<u64>, bool) {
    let Some(tool_call_id) = context.tool_call_id.as_deref() else {
        let (result, cpu_ns) = invoke.await;
        return (result, cpu_ns, false);
    };
    let registration = context
        .extension::<ToolCallCancels>()
        .unwrap_or_else(shared)
        .register(session_id, tool_call_id);
    let invoke = std::pin::pin!(invoke);
    let cancelled = std::pin::pin!(registration.token().cancelled());
    match futures::future::select(invoke, cancelled).await {
        futures::future::Either::Left(((result, cpu_ns), _)) => (result, cpu_ns, false),
        futures::future::Either::Right(_) => {
            if let Some(token) = &context.cancellation {
                token.cancel();
            }
            (Ok(cancelled_result(tool_call_id)), None, true)
        }
    }
}

/// The result a cancelled call completes with.
pub fn cancelled_result(tool_call_id: &str) -> ToolResult {
    ToolResult {
        tool_call_id: tool_call_id.to_string(),
        result: None,
        images: None,
        error: Some(CANCELLED_MESSAGE.to_string()),
        connection_required: None,
        raw_output: None,
        details: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_registered_call_cancels_once_and_is_forgotten_when_it_ends() {
        let cancels = Arc::new(ToolCallCancels::new());
        let session = SessionId::new();
        let registration = cancels.register(session, "call_1");
        assert!(cancels.is_running(session, "call_1"));
        assert!(!cancels.cancel(session, "call_2"), "unknown call");
        assert!(!cancels.cancel(SessionId::new(), "call_1"), "other session");
        assert!(cancels.cancel(session, "call_1"));
        assert!(registration.token().is_cancelled());
        assert!(!cancels.cancel(session, "call_1"), "already cancelled");
        drop(registration);
        assert!(!cancels.is_running(session, "call_1"));
    }

    #[test]
    fn an_older_registration_does_not_forget_a_newer_one() {
        let cancels = Arc::new(ToolCallCancels::new());
        let session = SessionId::new();
        let first = cancels.register(session, "call_1");
        let second = cancels.register(session, "call_1");
        drop(first);
        assert!(cancels.is_running(session, "call_1"));
        drop(second);
        assert!(!cancels.is_running(session, "call_1"));
    }
}
