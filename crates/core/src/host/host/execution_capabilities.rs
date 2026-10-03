use super::*;

pub(super) async fn load_execution_capabilities<A: RuntimeHostAdapter>(
    adapter: &A,
    org_id: i64,
    session_id: SessionId,
    harness_id: HarnessId,
    agent_id: Option<AgentId>,
    locale: Option<String>,
    blueprint_id: Option<&str>,
) -> everruns_contracts::error::Result<RuntimeExecutionCapabilities> {
    let capability_registry = adapter.capability_registry();
    if let Some(blueprint_id) = blueprint_id {
        let mut registry = ToolRegistry::with_defaults();
        #[cfg(feature = "builtins")]
        crate::builtins::register_default_tools(&mut registry);
        let blueprint = capability_registry.blueprint(blueprint_id).ok_or_else(|| {
            everruns_contracts::error::AgentLoopError::config(format!(
                "Blueprint \"{blueprint_id}\" not found in registry"
            ))
        })?;
        for tool in blueprint.tools {
            registry.register_boxed(tool);
        }
        return Ok(RuntimeExecutionCapabilities {
            tool_registry: registry,
            post_tool_hooks: Vec::new(),
            pre_tool_hooks: Vec::new(),
            tool_call_hooks: Vec::new(),
            subagent_nesting_policy: crate::delegation_services::SubagentNestingPolicy::default(),
            resolved_capabilities: Vec::new(),
        });
    }

    let harness = adapter
        .harness_store(org_id)
        .get_harness(harness_id)
        .await?
        .ok_or_else(|| everruns_contracts::error::AgentLoopError::harness_not_found(harness_id))?;

    let session = adapter
        .session_store(org_id)
        .get_session(session_id)
        .await?
        .ok_or_else(|| everruns_contracts::error::AgentLoopError::session_not_found(session_id))?;

    let agent_store = adapter.agent_store(org_id);
    let agent =
        match agent_id {
            Some(agent_id) => Some(agent_store.get_agent(agent_id).await?.ok_or_else(|| {
                everruns_contracts::error::AgentLoopError::agent_not_found(agent_id)
            })?),
            None => None,
        };

    let resolved =
        resolve_runtime_capabilities(&harness, agent.as_ref(), &session, &capability_registry);
    // Executor (act) path: this builds the worker-side tool registry, not the
    // model-visible tool list. The model is left unset, so a model-adaptive
    // capability like `auto_tool_search` resolves to its provider-agnostic
    // client-side mechanism here. That registers the `tool_search` tool in the
    // executor, which is a harmless superset: on native models the reason path
    // never shows that tool to the model, so it is simply never called.
    let prompt_ctx = SystemPromptContext {
        session_id,
        locale: locale.or(session.locale.clone()),
        // Pin system-prompt file reads to the session's workspace (the default
        // 1:1 case is a transparent pass-through), then resolve through the
        // mount resolver (EVE-660): `/workspace` is a mount + cwd.
        // `scoped_prompt_file_store` wraps with `wrap_if_needed` so a local
        // embedder's backend-native display policy survives here too (it must
        // match the reason path — see its doc); server stores stay on `/workspace`.
        file_store: Some(crate::scoped_prompt_file_store(
            adapter.file_store(org_id),
            session.workspace_id,
        )),
        model: None,
        session_storage: None,
    };
    let collected = collect_capabilities_with_configs(
        &resolved.resolved_capability_configs,
        &capability_registry,
        &prompt_ctx,
    )
    .await;

    let mut registry = ToolRegistry::with_defaults();
    #[cfg(feature = "builtins")]
    crate::builtins::register_default_tools(&mut registry);
    for tool in collected.tools {
        registry.register_boxed(tool);
    }

    // Only `Available` capabilities contribute hooks, matching
    // `collect_capabilities_with_configs` (which skips non-available
    // capabilities). This keeps a `ComingSoon`/unavailable capability from
    // affecting execution via any of its hook seams.
    let mut post_tool_hooks: Vec<Arc<dyn crate::tool_hooks::PostToolExecHook>> = resolved
        .resolved_capability_configs
        .iter()
        .flat_map(|config| {
            capability_registry
                .get(config.capability_id())
                .filter(|capability| capability.status().is_active())
                .map(|capability| {
                    capability.post_tool_exec_hooks_with_config(config.config_value())
                })
                .unwrap_or_default()
        })
        .collect();
    // Tool-output guardrails must inspect the original result before other
    // capability hooks can persist or compact it into secondary surfaces.
    post_tool_hooks.sort_by_key(|hook| hook.priority());

    // User-hook contributions (see `knowledge/runtime-resources/user-hooks.md`). `finalize_specs_from_configs`
    // gathers specs across every resolved capability — both the user-facing
    // `user_hooks` capability and any capability that bundles hooks — and applies
    // `finalize_hook_specs` (namespace stamping, stable ids, `disabled_contributions`
    // muting; TM-HOOK-004). The same helper backs the lifecycle firing points so
    // every event finalizes specs identically.
    let tool_augmentor = adapter.tool_augmentor();
    let user_hook_specs = finalize_specs_from_configs(
        &resolved.resolved_capability_configs,
        &capability_registry,
        tool_augmentor.as_deref(),
    );
    // Persisted messages remain the immutable audit record, so they can contain
    // text removed by a provider-bound user_prompt_submit hook. Until there is a
    // durable provider-visible history view, fail closed rather than let
    // query_history bypass that enforcement boundary.
    if user_hook_specs
        .iter()
        .any(|spec| spec.event == crate::user_hook_types::HookEvent::UserPromptSubmit)
    {
        registry.unregister("query_history");
    }
    // Capability-contributed pre-tool hooks run first (e.g. approval gating),
    // then user-hook (`PreToolUse`) specs. The first hook to block wins.
    let mut pre_tool_hooks: Vec<Arc<dyn crate::tool_hooks::PreToolUseHook>> = resolved
        .resolved_capability_configs
        .iter()
        .flat_map(|config| {
            capability_registry
                .get(config.capability_id())
                .filter(|capability| capability.status().is_active())
                .map(|capability| capability.pre_tool_use_hooks_with_config(config.config_value()))
                .unwrap_or_default()
        })
        .collect();
    if !user_hook_specs.is_empty() {
        let dispatcher = bash_hook_dispatcher(adapter.file_store(org_id));
        post_tool_hooks.extend(crate::hook_adapter::build_post_tool_use_hooks(
            &user_hook_specs,
            dispatcher.clone(),
        ));
        pre_tool_hooks.extend(crate::hook_adapter::build_pre_tool_use_hooks(
            &user_hook_specs,
            dispatcher,
        ));
    }

    // Use the hook list assembled by `collect_capabilities_with_configs` as the
    // single source of truth. It already contains every explicit capability
    // `tool_call_hooks()` followed by the generated `CapabilityNarrationHook`
    // adapters — one per collected capability plus any auto-activated
    // cross-cutting capability such as `background_execution`. Re-deriving only
    // the explicit subset here dropped capability-owned narration, so tools fell
    // back to generic `Ran {display_name}` lines (EVE-601). Explicit hooks stay
    // first in this list, so model-authored narration (`human_intent`) keeps its
    // precedence over default `Tool::narrate()`, and only available capabilities
    // contributed because collection skips non-available ones.
    let tool_call_hooks = collected.tool_call_hooks;

    Ok(RuntimeExecutionCapabilities {
        tool_registry: registry,
        post_tool_hooks,
        pre_tool_hooks,
        tool_call_hooks,
        subagent_nesting_policy: subagent_nesting_policy_from_configs(
            &resolved.resolved_capability_configs,
        ),
        resolved_capabilities: resolved.resolved_capability_configs,
    })
}
