//! External implementations of narrow core and host execution contracts.

use async_trait::async_trait;
use everruns_core::event_emitter::EventEmitter;
use everruns_core::events::{Event, EventRequest};
use everruns_core::session_files::SessionFileSystem;
use everruns_core::{AgentLoopError, Result};
use everruns_host::{SessionFileSystemFactory, SessionFileSystemFactoryContext};
use std::sync::Arc;

/// Minimal external observer proving event emission is independently implementable.
pub struct ExternalEventEmitter;

#[async_trait]
impl EventEmitter for ExternalEventEmitter {
    async fn emit(&self, request: EventRequest) -> Result<Event> {
        Ok(request.into_event(everruns_core::typed_id::EventId::new(), 1))
    }
}

/// External host factory proving deployment composition does not leak into core.
pub struct ExternalSessionFileSystemFactory;

#[async_trait]
impl SessionFileSystemFactory for ExternalSessionFileSystemFactory {
    fn name(&self) -> &'static str {
        "external"
    }

    async fn create_session_file_system(
        &self,
        _context: SessionFileSystemFactoryContext,
    ) -> Result<Arc<dyn SessionFileSystem>> {
        Err(AgentLoopError::config(
            "external fixture does not configure a filesystem backend",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_event_contract<T: EventEmitter>() {}
    fn assert_factory_contract<T: SessionFileSystemFactory>() {}

    #[test]
    fn narrow_contracts_are_public_without_a_platform_dependency() {
        assert_event_contract::<ExternalEventEmitter>();
        assert_factory_contract::<ExternalSessionFileSystemFactory>();
        assert_eq!(ExternalSessionFileSystemFactory.name(), "external");
    }
}
