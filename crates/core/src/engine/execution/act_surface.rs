//! Runtime service and tool-surface configuration for the act phase.

use super::*;

impl<T, E> ActAtom<T, E>
where
    T: ToolExecutor,
    E: PhaseEffectSink,
{
    /// Replace the complete runtime-owned service snapshot used for every
    /// per-call [`ToolContext`](crate::ToolContext). Production hosts should prefer this over
    /// assembling individual services on the atom.
    pub fn with_context_services(
        mut self,
        services: crate::engine::tool_context::ToolContextServices,
    ) -> Self {
        self.context_services = services;
        self
    }

    /// Set the resolved operation routes shared with nested shell adapters.
    pub fn with_tool_surface_plan(
        mut self,
        plan: everruns_contracts::runtime::tool_surface::ToolSurfacePlan,
    ) -> Self {
        self.tool_surface_plan = plan;
        self
    }
}
