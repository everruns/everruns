//! Stability: alpha.
//! Durable event routing. Applications own ingress authentication, scheduling and Engine hosting.
//!
//! A SQLite outbox retains input/session identities before dispatch. Recovering it
//! reuses Engine admission identities, including after an uncertain response.
use crate::{Engine, MessageId, SessionId, hosting::AgentBuild};
/// Filter conditions use the same semantics as Server triggers.
pub use everruns_contracts::runtime::triggers::{
    TriggerChangesOnly as ChangesOnly, TriggerEventFilter as Filter,
    TriggerFilterCondition as Condition,
};
use everruns_core::triggers::{
    change_miss, filter_miss, render_message_template_bounded, template_lookup,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc};
mod store;
pub use store::{SqliteStore, StateStore, VersionedState};
mod sources;
pub use sources::{GitHubRequest, McpRequest, SignedHeaders, github, mcp};
/// A normalized event, after ingress has authenticated it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    /// Stable delivery id. Required for deduplication.
    pub id: String,
    /// Source name.
    pub source: String,
    /// Source event type.
    pub kind: String,
    /// Subject used for conversation routing and changes-only filters.
    pub subject: Option<String>,
    /// Template/filter data.
    pub context: Value,
}
/// Session routing policy.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub enum Routing {
    /// A fresh session for each delivery.
    #[default]
    PerEvent,
    /// One session per event subject; missing subjects use a fresh session.
    PerSubject,
    /// One shared session for this trigger revision.
    Shared,
}
/// Immutable trigger configuration. Change `key` when its behavior changes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trigger {
    /// Stable application-scoped identity including revision.
    pub key: String,
    /// Exact Engine registration.
    pub build: AgentBuild,
    /// User message, with `{{ dotted.paths }}` substitutions.
    pub message: String,
    /// Conditions and optional change detection.
    pub filter: Filter,
    /// Conversation policy.
    pub routing: Routing,
}
/// Trigger processing failure. No provider credentials are included.
#[derive(Debug, thiserror::Error)]
#[error("trigger: {0}")]
pub struct Error(pub String);
/// Result of processing an event. Dispatch means durably admitted, not completed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Accepted by Engine under this stable identity.
    Dispatched {
        /// Session that received the event.
        session: SessionId,
        /// Input identity reusable for waiting or inspection.
        input: MessageId,
    },
    /// Already accepted or filtered; durable receipt tombstones prevent replays after history pruning.
    Duplicate,
    /// No session was started.
    Filtered {
        /// The first rejected condition.
        reason: String,
    },
}
/// Persisted outbox state. Treat as private application data.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct State {
    /// Configuration fingerprint, prevents accidental key reuse.
    pub configuration: String,
    /// Pending and recently delivered events, in acceptance order.
    pub deliveries: Vec<Delivery>,
    /// Latest value per subject (up to 500).
    pub subjects: BTreeMap<String, SubjectValue>,
}
/// Latest observed subject value, with its acceptance revision for bounded eviction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubjectValue {
    /// Value used for change detection.
    pub value: Value,
    /// Last state revision that observed this subject.
    pub last_seen: i64,
}
/// A retained outbox delivery. Never mutate independently of StateStore's CAS.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Delivery {
    /// Source and delivery identity.
    pub key: String,
    /// Stable session.
    pub session: SessionId,
    /// Stable input.
    pub input: MessageId,
    /// Frozen rendered input.
    pub message: String,
    /// Whether Engine committed admission.
    pub admitted: bool,
    /// Filter rejection when this delivery starts no session.
    pub filtered: Option<String>,
}
/// Event pipeline over an Engine and an atomic application state store.
#[derive(Clone)]
pub struct Dispatcher {
    engine: Engine,
    store: Arc<dyn StateStore>,
    trigger: Trigger,
}
impl Dispatcher {
    /// Bind one immutable trigger to its Engine and store.
    pub fn new(
        engine: &Engine,
        store: impl StateStore + 'static,
        trigger: Trigger,
    ) -> Result<Self, Error> {
        if trigger.key.trim().is_empty()
            || trigger.key.len() > 512
            || trigger.message.len() > 64 * 1024
        {
            return Err(Error("Invalid trigger key or message".into()));
        }
        if trigger.filter.conditions.len() > 50 {
            return Err(Error("Too many trigger conditions".into()));
        }
        Ok(Self {
            engine: engine.clone(),
            store: Arc::new(store),
            trigger,
        })
    }
    fn configuration(&self) -> Result<String, Error> {
        serde_json::to_string(&self.trigger)
            .map_err(|_| Error("Invalid trigger configuration".into()))
    }
    /// Filter, atomically retain, then admit one event. Concurrent retries use the same identities.
    pub async fn dispatch(&self, mut event: Event) -> Result<Outcome, Error> {
        if event.id.trim().is_empty()
            || event.id.len() > 512
            || event.source.is_empty()
            || event.source.len() > 64
            || event.subject.as_ref().is_some_and(|s| s.len() > 512)
            || event.context.to_string().len() > 256 * 1024
        {
            return Err(Error("Invalid event or event exceeds limits".into()));
        }
        if !event.context.is_object() {
            return Err(Error("Event context must be an object".into()));
        }
        event.context["event"] =
            json!({"id":event.id,"source":event.source,"type":event.kind,"subject":event.subject});
        let key = serde_json::to_string(&(event.source.clone(), event.id.clone()))
            .map_err(|_| Error("Invalid delivery key".into()))?;
        for _ in 0..32 {
            let versioned = self.store.load(&self.trigger.key).await?;
            let mut state = versioned.state;
            let config = self.configuration()?;
            if !state.configuration.is_empty() && state.configuration != config {
                return Err(Error(
                    "Trigger key belongs to another configuration; use a new revision".into(),
                ));
            }
            state.configuration = config;
            if self.store.seen(&self.trigger.key, &key).await?
                && !state.deliveries.iter().any(|d| d.key == key && !d.admitted)
            {
                return Ok(Outcome::Duplicate);
            }
            if let Some(delivery) = state.deliveries.iter().find(|d| d.key == key) {
                if delivery.admitted {
                    return Ok(Outcome::Duplicate);
                }
                return self.admit(delivery.clone()).await;
            }
            let mut miss = filter_miss(&self.trigger.filter, &event.context);
            if let Some(change) = &self.trigger.filter.changes_only {
                let subject = event
                    .subject
                    .as_ref()
                    .ok_or_else(|| Error("Changes-only events require a subject".into()))?;
                let current = template_lookup(&event.context, &change.path);
                let change_reason = change_miss(
                    &change.path,
                    current,
                    state.subjects.get(subject).map(|s| &s.value),
                );
                if let Some(value) = current {
                    state.subjects.insert(
                        subject.clone(),
                        SubjectValue {
                            value: value.clone(),
                            last_seen: versioned.revision,
                        },
                    );
                }
                while state.subjects.len() > 500 {
                    let Some(first) = state
                        .subjects
                        .iter()
                        .min_by_key(|(_, value)| value.last_seen)
                        .map(|(key, _)| key.clone())
                    else {
                        break;
                    };
                    state.subjects.remove(&first);
                }
                if miss.is_none() {
                    miss = change_reason;
                }
            }
            if let Some(reason) = miss {
                state.deliveries.push(Delivery {
                    key: key.clone(),
                    session: SessionId::new(),
                    input: MessageId::new(),
                    message: String::new(),
                    admitted: true,
                    filtered: Some(reason.clone()),
                });
                while state.deliveries.len() > 2000 {
                    if let Some(i) = state.deliveries.iter().position(|d| d.admitted) {
                        state.deliveries.remove(i);
                    } else {
                        break;
                    }
                }

                if self
                    .store
                    .compare_exchange(&self.trigger.key, versioned.revision, &state, Some(&key))
                    .await?
                {
                    return Ok(Outcome::Filtered { reason });
                }
                continue;
            }
            if state.deliveries.iter().filter(|d| !d.admitted).count() >= 200 {
                return Err(Error(
                    "Pending outbox is full; recover before accepting more work".into(),
                ));
            }
            let route = match self.trigger.routing {
                Routing::Shared => "shared".to_string(),
                Routing::PerSubject => event
                    .subject
                    .clone()
                    .map(|s| format!("subject:{s}"))
                    .unwrap_or_else(|| format!("event:{key}")),
                Routing::PerEvent => format!("event:{key}"),
            };
            let namespace =
                uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, self.trigger.key.as_bytes());
            let delivery = Delivery {
                key: key.clone(),
                session: SessionId::from_uuid(uuid::Uuid::new_v5(&namespace, route.as_bytes())),
                input: MessageId::from_uuid(uuid::Uuid::new_v5(&namespace, key.as_bytes())),
                message: render_message_template_bounded(
                    &self.trigger.message,
                    &event.context,
                    64 * 1024,
                )
                .ok_or_else(|| Error("Rendered trigger message exceeds limit".into()))?,
                admitted: false,
                filtered: None,
            };
            state.deliveries.push(delivery.clone());
            while state.deliveries.len() > 2000 {
                if let Some(i) = state.deliveries.iter().position(|d| d.admitted) {
                    state.deliveries.remove(i);
                } else {
                    break;
                }
            }
            // THREAT[TM-FW-005]: commit the frozen input and receipt before Engine admission.
            if self
                .store
                .compare_exchange(&self.trigger.key, versioned.revision, &state, Some(&key))
                .await?
            {
                return self.admit(delivery).await;
            }
        }
        Err(Error("Trigger state is busy; retry this delivery".into()))
    }
    async fn admit(&self, delivery: Delivery) -> Result<Outcome, Error> {
        let session = self
            .engine
            .create_registered(delivery.session, &self.trigger.build)
            .await
            .map_err(|_| Error("Could not open registered trigger session".into()))?;
        session
            .send_with_id(delivery.input, delivery.message.clone())
            .await
            .map_err(|_| {
                Error("Could not admit trigger input; delivery retained for recovery".into())
            })?;
        for _ in 0..32 {
            let mut versioned = self.store.load(&self.trigger.key).await?;
            if let Some(d) = versioned
                .state
                .deliveries
                .iter_mut()
                .find(|d| d.key == delivery.key)
            {
                d.admitted = true;
            }
            if self
                .store
                .compare_exchange(
                    &self.trigger.key,
                    versioned.revision,
                    &versioned.state,
                    None,
                )
                .await?
            {
                return Ok(Outcome::Dispatched {
                    session: delivery.session,
                    input: delivery.input,
                });
            }
        }
        Err(Error(
            "Input admitted but outbox is busy; retry recovery".into(),
        ))
    }
    /// Retry every retained unadmitted input, with its original identity and text.
    /// Call at startup and from the application's scheduling loop.
    pub async fn recover(&self) -> Result<Vec<Outcome>, Error> {
        let state = self.store.load(&self.trigger.key).await?.state;
        if !state.configuration.is_empty() && state.configuration != self.configuration()? {
            return Err(Error("Trigger configuration changed".into()));
        }
        let mut outcomes = vec![];
        for d in state.deliveries.into_iter().filter(|d| !d.admitted) {
            outcomes.push(self.admit(d).await?)
        }
        Ok(outcomes)
    }
}
