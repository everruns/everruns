use std::sync::Arc;

use everruns_core::MessageRetriever;
use everruns_core::atoms::ReasonAtom;
use everruns_core::capabilities::CapabilityRegistry;
use everruns_core::driver_registry::DriverRegistry;
use everruns_core::event_emitter::EventEmitter;
use everruns_core::execution_loading::{AgentStore, HarnessStore, SessionStore};
use everruns_core::provider_resolution::ProviderStore;
use everruns_host::StoreTurnContextResolver;

/// Compose a reason atom from in-memory stores for tests and examples.
///
/// Production code should construct [`StoreTurnContextResolver`] in its host
/// composition and pass the resulting narrow resolver to [`ReasonAtom`].
#[allow(clippy::too_many_arguments)]
pub fn reason_atom_with_stores<H, A, S, M, P, E>(
    harness_store: H,
    agent_store: A,
    session_store: S,
    message_retriever: M,
    provider_store: P,
    capability_registry: CapabilityRegistry,
    driver_registry: DriverRegistry,
    event_emitter: E,
) -> ReasonAtom
where
    H: HarnessStore + 'static,
    A: AgentStore + 'static,
    S: SessionStore + 'static,
    M: MessageRetriever + Clone + 'static,
    P: ProviderStore + 'static,
    E: EventEmitter + 'static,
{
    let context_resolver = StoreTurnContextResolver::new(
        Arc::new(harness_store),
        Arc::new(agent_store),
        Arc::new(session_store),
        Arc::new(message_retriever.clone()),
        Arc::new(provider_store),
        capability_registry.clone(),
        driver_registry,
    );

    ReasonAtom::new(
        context_resolver,
        message_retriever,
        capability_registry,
        event_emitter,
    )
}
