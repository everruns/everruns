//! `Cx`: the one context type, handed to tools, schedules and evals.
//!
//! Like a Topcoat component fetching its own data through `&Cx`, a tool asks
//! the context for what it needs (a connection, a secret, a person's approval)
//! instead of having it threaded through the agent definition.

use std::any::Any;
use std::future::{Future, IntoFuture};
use std::pin::Pin;
use std::sync::{Arc, OnceLock, Weak};

use anyhow::anyhow;
use serde_json::{Value, json};

use crate::app::Mode;
use crate::host::Host;

/// The context of one session (inside tools) or of the app (in schedules).
#[derive(Clone)]
pub struct Cx {
    host: Weak<Host>,
    session: Arc<OnceLock<String>>,
    agent: Option<&'static str>,
    tool: Option<&'static str>,
}

impl std::fmt::Debug for Cx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cx")
            .field("session", &self.session.get())
            .field("agent", &self.agent)
            .field("tool", &self.tool)
            .finish()
    }
}

impl Cx {
    /// App-level context (schedules): no session.
    pub(crate) fn app(host: &Arc<Host>) -> Self {
        Self {
            host: Arc::downgrade(host),
            session: Arc::new(OnceLock::new()),
            agent: None,
            tool: None,
        }
    }

    /// Context for a session whose id is filled in once everruns assigns it.
    pub(crate) fn session(
        host: &Arc<Host>,
        agent: &'static str,
        id: Arc<OnceLock<String>>,
    ) -> Self {
        Self {
            host: Arc::downgrade(host),
            session: id,
            agent: Some(agent),
            tool: None,
        }
    }

    pub(crate) fn for_tool(&self, tool: &'static str) -> Self {
        Self {
            tool: Some(tool),
            ..self.clone()
        }
    }

    fn host(&self) -> crate::Result<Arc<Host>> {
        self.host
            .upgrade()
            .ok_or_else(|| anyhow!("the serve host has shut down"))
    }

    /// The current session id, inside a tool.
    pub fn session_id(&self) -> Option<&str> {
        self.session.get().map(String::as_str)
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

    /// Report progress. Appears as a `tool.progress` event on the session's
    /// stream and in the dev console.
    pub fn progress(&self, message: impl Into<String>) {
        let (Ok(host), Some(session)) = (self.host(), self.session_id()) else {
            return;
        };
        host.emit(
            session,
            "tool.progress",
            json!({ "tool": self.tool, "message": message.into() }),
        );
    }

    /// Start a new session, e.g. from a schedule. Awaiting it runs the first
    /// turn and delivers the reply to [`deliver_to`](StartSession::deliver_to).
    pub fn start_session(&self, input: impl Into<String>) -> StartSession {
        StartSession {
            cx: self.clone(),
            input: input.into(),
            agent: None,
            deliver_to: None,
            metadata: Value::Null,
        }
    }

    /// Ask a person to approve a tool call and wait for the decision.
    pub(crate) async fn approval(&self, tool: &str, arguments: &Value) -> crate::Result<Decision> {
        let host = self.host()?;
        let session = self
            .session_id()
            .ok_or_else(|| anyhow!("approvals need a session"))?
            .to_string();
        host.request_approval(&session, tool, arguments).await
    }
}

/// A person's answer to an approval request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Decision {
    Approve,
    Deny { note: Option<String> },
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
    metadata: Value,
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
        self.metadata = metadata;
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
                .create_session(
                    self.agent.as_deref(),
                    self.metadata,
                    self.deliver_to.map(|target| target.encode()),
                )
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
