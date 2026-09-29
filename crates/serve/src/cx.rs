//! `Cx`: the one context type, handed to tools, schedules and evals.
//!
//! Like a Topcoat component fetching its own data through `&Cx`, a tool asks
//! the context for what it needs (a connection, a secret, progress reporting)
//! instead of having it threaded through the agent definition. Approvals are
//! declared on the tool (`#[tool(needs_approval …)]`) and enforced by the
//! everruns runtime, not asked for from inside the tool body.

use std::any::Any;
use std::future::{Future, IntoFuture};
use std::pin::Pin;
use std::sync::{Arc, Weak};

use anyhow::anyhow;
use everruns::ToolCallContext;
use serde_json::Value;

use crate::app::Mode;
use crate::host::{Host, NewSession};

/// The context of one tool call (inside tools) or of the app (in schedules).
///
/// Inside a tool it wraps the runtime's `everruns::ToolCallContext` (session,
/// turn and tool call ids, progress) and adds host access: connections and
/// secrets.
#[derive(Clone)]
pub struct Cx {
    host: Weak<Host>,
    agent: Option<&'static str>,
    session: Option<String>,
    call: Option<ToolCallContext>,
}

impl std::fmt::Debug for Cx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cx")
            .field("session", &self.session)
            .field("agent", &self.agent)
            .field("call", &self.call)
            .finish()
    }
}

impl Cx {
    /// App-level context (schedules): no session.
    pub(crate) fn app(host: &Arc<Host>) -> Self {
        Self {
            host: Arc::downgrade(host),
            agent: None,
            session: None,
            call: None,
        }
    }

    /// Context for one tool call.
    pub(crate) fn tool(host: Weak<Host>, agent: &'static str, call: ToolCallContext) -> Self {
        Self {
            host,
            agent: Some(agent),
            session: Some(call.session_id().to_string()),
            call: Some(call),
        }
    }

    fn host(&self) -> crate::Result<Arc<Host>> {
        self.host
            .upgrade()
            .ok_or_else(|| anyhow!("the serve host has shut down"))
    }

    /// The current session id, inside a tool.
    pub fn session_id(&self) -> Option<&str> {
        self.session.as_deref()
    }

    /// The current tool call id, inside a tool.
    pub fn tool_call_id(&self) -> Option<&str> {
        self.call.as_ref().map(ToolCallContext::tool_call_id)
    }

    /// The turn that made the current call, inside a tool.
    pub fn turn_id(&self) -> Option<String> {
        self.call.as_ref().and_then(ToolCallContext::turn_id)
    }

    /// The agent this context belongs to, inside a tool.
    pub fn agent(&self) -> Option<&'static str> {
        self.agent
    }

    /// How the binary is running.
    pub fn mode(&self) -> Option<Mode> {
        self.host.upgrade().map(|host| host.mode)
    }

    /// The `#[connection]` of type `T`. The value (and any credentials in it)
    /// never reaches the model.
    pub fn connection<T: Any + Send + Sync>(&self) -> crate::Result<Arc<T>> {
        let host = self.host()?;
        host.app
            .inner
            .connections
            .iter()
            .find_map(|entry| entry.value.value.clone().downcast::<T>().ok())
            .ok_or_else(|| anyhow!("no #[connection] returns `{}`", std::any::type_name::<T>()))
    }

    /// A host-provided secret by name.
    pub fn secret(&self, name: &'static str) -> crate::Result<String> {
        crate::Secret::named(name).value()
    }

    /// Report progress. Appears as a canonical `tool.progress` event on the
    /// session's stream and in the dev console. A no-op outside a tool.
    pub async fn progress(&self, message: impl Into<String>) {
        if let Some(call) = &self.call {
            call.progress(message).await;
        }
    }

    /// Start a new session, e.g. from a schedule. Awaiting it runs the first
    /// turn and delivers the reply to [`deliver_to`](StartSession::deliver_to).
    pub fn start_session(&self, input: impl Into<String>) -> StartSession {
        StartSession {
            cx: self.clone(),
            input: input.into(),
            agent: None,
            deliver_to: None,
            metadata: None,
        }
    }
}

/// Where a session's replies go: a channel and a target on it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeliveryTarget {
    pub channel: String,
    pub target: String,
}

impl DeliveryTarget {
    pub fn new(channel: impl Into<String>, target: impl Into<String>) -> Self {
        Self {
            channel: channel.into(),
            target: target.into(),
        }
    }

    /// `channel:target`, as stored with the session.
    pub(crate) fn encode(&self) -> String {
        format!("{}:{}", self.channel, self.target)
    }

    pub(crate) fn decode(value: &str) -> Option<Self> {
        let (channel, target) = value.split_once(':')?;
        Some(Self::new(channel, target))
    }
}

/// A session being started from code. Configure, then `.await`.
#[must_use = "a StartSession does nothing until awaited"]
pub struct StartSession {
    cx: Cx,
    input: String,
    agent: Option<String>,
    deliver_to: Option<DeliveryTarget>,
    metadata: Option<Value>,
}

impl StartSession {
    /// Run on this agent instead of the default one.
    pub fn agent(mut self, name: impl Into<String>) -> Self {
        self.agent = Some(name.into());
        self
    }

    /// Deliver every reply of this session to a channel.
    pub fn deliver_to(mut self, target: DeliveryTarget) -> Self {
        self.deliver_to = Some(target);
        self
    }

    /// Attach metadata, visible on the session.
    pub fn metadata(mut self, metadata: Value) -> Self {
        self.metadata = Some(metadata);
        self
    }
}

impl IntoFuture for StartSession {
    type Output = crate::Result;
    type IntoFuture = Pin<Box<dyn Future<Output = crate::Result> + Send>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let host = self.cx.host()?;
            let session = host
                .create_session(NewSession {
                    agent: self.agent,
                    metadata: self.metadata,
                    deliver_to: self.deliver_to.map(|target| target.encode()),
                    ..NewSession::default()
                })
                .await?;
            let turn = host.send(&session, self.input).await?;
            let outcome = turn.wait().await?;
            if !outcome.success {
                anyhow::bail!(
                    "session {session} turn failed: {}",
                    outcome.error.unwrap_or_default()
                );
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delivery_target_keeps_colons_in_the_target() {
        let target = DeliveryTarget::new("slack", "C1:1700000000.0001");
        let decoded = DeliveryTarget::decode(&target.encode()).unwrap();
        assert_eq!(decoded, target);
    }
}
