use super::*;

impl ReasonAtom {
    /// Causal input identities for this phase, persisted before provider work.
    pub fn with_causal_context(mut self, context: crate::causal::CausalContext) -> Self {
        self.causal_context = context;
        self
    }

    pub fn with_native_async(
        mut self,
        coordinator: Arc<tokio::sync::Mutex<crate::engine::native_async::NativeAsyncCoordinator>>,
    ) -> Self {
        self.native_async = Some(coordinator);
        self
    }
}
