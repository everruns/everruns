//! Everruns 0.17 compatibility adapter.
//!
//! All maintained host execution lives in [`everruns_host`]. This crate keeps
//! established `everruns-runtime` paths source-compatible where practical by
//! re-exporting that one canonical implementation. New ordinary applications
//! should use [`everruns`](https://docs.rs/everruns); advanced host integrators
//! may depend on [`everruns-host`](https://docs.rs/everruns-host) directly. Both
//! are part of the [Everruns](https://everruns.com) ecosystem.
//!
//! # Example
//!
//! ```
//! use everruns_runtime::{InProcessRuntimeBuilder, RuntimeBackends};
//!
//! let builder = InProcessRuntimeBuilder::new().backends(RuntimeBackends::in_memory());
//! # let _ = builder;
//! ```

use std::sync::Arc;

use async_trait::async_trait;
use everruns_core::error::Result;
use everruns_core::events::Event;
use everruns_core::message::Message;
use everruns_core::message_retriever::{InputMessage, MessageRetriever};
use everruns_core::traits::EventEmitter;
use everruns_core::typed_id::SessionId;

/// 0.17 name for the canonical host backend bundle.
pub use everruns_host::HostBackends as RuntimeBackends;
pub use everruns_host::*;

/// Legacy writable message-store contract retained only for 0.17 source paths.
///
/// It is not accepted by [`RuntimeBackends`] and is never used by execution.
/// Canonical events are the only maintained write path; this shim is scheduled
/// for deletion in 0.18.
#[deprecated(
    since = "0.17.25",
    note = "execution is event-authoritative; use everruns_host::EventLog and EventHistory"
)]
#[async_trait]
pub trait RuntimeMessageStore: MessageRetriever + Send + Sync {
    /// Legacy, non-authoritative input-message write.
    async fn add_input_message(
        &self,
        session_id: SessionId,
        input: InputMessage,
    ) -> Result<Message>;

    /// Legacy, non-authoritative message write.
    async fn store_message(&self, session_id: SessionId, message: Message) -> Result<()>;
}

#[allow(deprecated)]
#[async_trait]
impl RuntimeMessageStore for everruns_core::in_memory::InMemoryMessageRetriever {
    async fn add_input_message(
        &self,
        session_id: SessionId,
        input: InputMessage,
    ) -> Result<Message> {
        self.add(session_id, input).await
    }

    async fn store_message(&self, session_id: SessionId, message: Message) -> Result<()> {
        self.store(session_id, message).await
    }
}

/// Legacy observation trait retained only so 0.17 implementations still name
/// the path. It is outside maintained execution; use [`EventSink`] for live
/// post-commit observation and [`EventReader`] for replay.
#[deprecated(
    since = "0.17.25",
    note = "use everruns_host::EventSink and EventReader"
)]
#[async_trait]
pub trait EventBus: EventEmitter {
    /// Legacy collected-event view.
    async fn collected_events(&self) -> Vec<Event> {
        Vec::new()
    }
}

#[allow(deprecated)]
#[async_trait]
impl<T: EventBus + ?Sized> EventBus for Arc<T> {
    async fn collected_events(&self) -> Vec<Event> {
        (**self).collected_events().await
    }
}

#[allow(deprecated)]
#[async_trait]
impl EventBus for everruns_core::in_memory::InMemoryEventEmitter {
    async fn collected_events(&self) -> Vec<Event> {
        self.events().await
    }
}
