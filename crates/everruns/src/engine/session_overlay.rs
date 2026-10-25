//! Precedence between a trusted Agent snapshot and a persisted session record
//! when the Engine (re)builds a runtime (EVE-1299).
//!
//! Every attach, resume, inspection and restart rebuilds the in-process runtime
//! from the application's Agent value, and that build writes the session record
//! back. The record mixes two owners, so each field has one explicit rule:
//!
//! - **Agent/Engine-owned, re-derived on every build.** Association ids
//!   (`organization_id`, `workspace_id`, `harness_id`, `agent_id`) name the
//!   definitions and environment binding this build just produced; copying them
//!   from an older record would bind the session to a definition that no longer
//!   exists. Agent configuration (`mcp_servers`, `initial_files`,
//!   `network_access`, `parallel_tool_calls`) comes only from the trusted Agent,
//!   so a server or file removed from it does not survive in old sessions.
//!   Run-scoped fields (`tools`, `system_prompt`) are replaced before each run
//!   by their owners (AG-UI and hosted session configuration) and start from the
//!   Agent's values. `status` is runtime state the in-process host re-derives.
//! - **Session-owned, the persisted value wins.** Catalog and lineage
//!   (`title`, `goal`, `locale`, `tags`, `parent_session_id`,
//!   `forked_from_session_id`), session-level overrides (`model_id`,
//!   `max_iterations`), blueprint selection, cumulative `usage`, and client
//!   hints other than the Agent-derived ones. A fresh build never writes these,
//!   so the persisted value is the only source.
//! - **Capabilities.** Agent capabilities live on the agent (and synthetic
//!   harness) layer, not in the session record, so the record's `capabilities`
//!   are exactly the session overlay: capabilities hot-enabled on this session.
//!   The overlay is kept as persisted. A deactivated overlay is absent from the
//!   record and therefore stays disabled; Agent defaults are not something a
//!   session can disable (`deactivate_capability` refuses inherited ones).
//!   Records written before this rule copied the Agent's capabilities into the
//!   session layer; entries the current Agent declares are dropped from the
//!   overlay so the Agent's current configuration wins instead of a stale copy.
//!   An entry an older Agent declared but the current one does not is
//!   indistinguishable from a hot-enabled overlay in such a record and is kept.
//! - **Live session controls (EVE-1297).** `Session::activate_capability` and
//!   `deactivate_capability` write exactly the capability overlay above.
//!   Live MCP edits are trusted Engine-held input: the runtime builder applies
//!   them over the Agent-derived `mcp_servers` after this reconciliation, so a
//!   credential-bearing definition is never copied from a record. Live model
//!   and reasoning-effort selections are not record fields (`model_id` stays
//!   whatever its session owner wrote): coherent hosting retains them in its
//!   authority `SessionConfiguration`, and the legacy lane holds them in the
//!   owning Engine. See `crate::session::controls`.

use std::collections::HashSet;

use everruns_contracts::CapabilityRef;
use everruns_core::host::RuntimeSessionStore;
use everruns_core::session::ExecutionSession;

/// Client hints the runtime builder derives from the Agent on every build.
/// Persisted values are discarded so the hints match the re-derived tools.
pub(crate) const AGENT_DERIVED_HINTS: [&str; 2] = ["setup_connection", "ask_user"];

/// Merge a freshly seeded session layer with the persisted record, if any.
///
/// `agent_capabilities` are the capabilities the trusted Agent contributes on
/// its own layer; see the module docs for every field's precedence.
pub(crate) async fn reconcile_session_record(
    store: &dyn RuntimeSessionStore,
    fresh: ExecutionSession,
    agent_capabilities: &[CapabilityRef],
) -> ExecutionSession {
    match store.get_session(fresh.id).await {
        Ok(Some(existing)) => reconcile(fresh, existing, agent_capabilities),
        _ => fresh,
    }
}

pub(crate) fn reconcile(
    mut fresh: ExecutionSession,
    existing: ExecutionSession,
    agent_capabilities: &[CapabilityRef],
) -> ExecutionSession {
    // Exhaustive on purpose: a new field fails to compile until it is given a
    // precedence rule here.
    let ExecutionSession {
        id: _,
        organization_id: _,
        workspace_id: _,
        harness_id: _,
        agent_id: _,
        title,
        goal,
        locale,
        tags,
        model_id,
        capabilities,
        tools: _,
        mcp_servers: _,
        system_prompt: _,
        initial_files: _,
        hints,
        network_access: _,
        max_iterations,
        parallel_tool_calls: _,
        status: _,
        usage,
        parent_session_id,
        forked_from_session_id,
        blueprint_id,
        blueprint_config,
    } = existing;

    fresh.title = fresh.title.or(title);
    fresh.goal = fresh.goal.or(goal);
    fresh.locale = fresh.locale.or(locale);
    if fresh.tags.is_empty() {
        fresh.tags = tags;
    }
    fresh.model_id = fresh.model_id.or(model_id);
    fresh.max_iterations = fresh.max_iterations.or(max_iterations);
    fresh.usage = fresh.usage.or(usage);
    fresh.parent_session_id = parent_session_id.or(fresh.parent_session_id);
    fresh.forked_from_session_id = forked_from_session_id.or(fresh.forked_from_session_id);
    fresh.blueprint_id = fresh.blueprint_id.or(blueprint_id);
    fresh.blueprint_config = fresh.blueprint_config.or(blueprint_config);

    // The lifecycle-hook capability is Engine-owned: the builder adds it only
    // when this build carries hook state, so a legacy copy is never an overlay.
    let agent_ids: HashSet<&str> = agent_capabilities
        .iter()
        .map(CapabilityRef::capability_id)
        .chain([crate::hooks::LIFECYCLE_HOOK_CAPABILITY_ID])
        .collect();
    let mut overlay = fresh.capabilities;
    for capability in capabilities {
        if !agent_ids.contains(capability.capability_id())
            && !overlay
                .iter()
                .any(|existing| existing.capability_id() == capability.capability_id())
        {
            overlay.push(capability);
        }
    }
    fresh.capabilities = overlay;

    if let Some(mut hints) = hints {
        hints.retain(|key, _| !AGENT_DERIVED_HINTS.contains(&key.as_str()));
        if let Some(fresh_hints) = fresh.hints.take() {
            hints.extend(fresh_hints);
        }
        fresh.hints = (!hints.is_empty()).then_some(hints);
    }
    fresh
}

#[cfg(test)]
#[path = "session_overlay_tests.rs"]
mod tests;
