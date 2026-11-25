//! Capability ids for agents that start other agents, and the rewrite of the
//! retired ids into them.
//!
//! Decision (2026-11-25): two ways to start work, never both on one agent.
//! `spawn_agents` makes one-off calls (brief in, result out, then done);
//! `threads` opens followed, continued work. `coordinator` is behaviour only
//! (a prompt) and needs `threads`. Every child talks to its parent through
//! tools the host attaches (`parent_communication`); children talk to each
//! other only with `sibling_communication`, which is opt-in.
//!
//! Decision: the retired ids (`subagents`, `agent_handoff`, `coordination`,
//! `run_conversation`) are rewritten, not aliased one to one. Two of them fold
//! into one new id with one config, so a plain alias would keep only the last
//! config. [`canonicalize_delegation_capabilities`] runs wherever capability
//! lists are resolved (`resolve_capability_configs`) and on every write path,
//! and the `threads_capability` migration rewrites stored rows the same way.

use serde_json::{Map, Value, json};

use super::CapabilityRef;

/// One-off calls to agents: `spawn_agent` plus the task tools.
pub const SPAWN_AGENTS_CAPABILITY_ID: &str = "spawn_agents";
/// Followed, continued work: `start_thread` and the thread tools.
pub const THREADS_CAPABILITY_ID: &str = "threads";
/// Coordinator behaviour: a prompt; needs `threads`.
pub const COORDINATOR_CAPABILITY_ID: &str = "coordinator";
/// Tools every child gets to talk to its parent. Attached by the host.
pub const PARENT_COMMUNICATION_CAPABILITY_ID: &str = "parent_communication";
/// Children of one run talk to each other. Opt-in.
pub const SIBLING_COMMUNICATION_CAPABILITY_ID: &str = "sibling_communication";

/// Retired: now `spawn_agents` with `self`.
pub const LEGACY_SUBAGENTS_ID: &str = "subagents";
/// Retired: now `spawn_agents` with `agents`.
pub const LEGACY_AGENT_HANDOFF_ID: &str = "agent_handoff";
/// Retired: now `threads` plus `coordinator`.
pub const LEGACY_COORDINATION_ID: &str = "coordination";
/// Retired: now `sibling_communication`.
pub const LEGACY_RUN_CONVERSATION_ID: &str = "run_conversation";

/// True for an id this module rewrites.
pub fn is_legacy_delegation_id(id: &str) -> bool {
    matches!(
        id,
        LEGACY_SUBAGENTS_ID
            | LEGACY_AGENT_HANDOFF_ID
            | LEGACY_COORDINATION_ID
            | LEGACY_RUN_CONVERSATION_ID
    )
}

/// Rewrite retired delegation ids into the current ones, in place of the first
/// retired entry. Idempotent; a list without retired ids is returned as is.
///
/// - `subagents` + `agent_handoff` become one `spawn_agents`: `self` when
///   `subagents` was there, `agents` from the hand-off members, limits from
///   the subagent config.
/// - `coordination` (coordinator role) becomes `threads` + `coordinator`:
///   `self`/`any` workers become `self`/`any_agent`, agent-id workers and the
///   agent's hand-off members become `agents`. Spawn agents are dropped then:
///   one agent never has both.
/// - `coordination` with the worker role is dropped: every thread gets the
///   parent tools from the host.
/// - `run_conversation` becomes `sibling_communication`.
pub fn canonicalize_delegation_capabilities(capabilities: &[CapabilityRef]) -> Vec<CapabilityRef> {
    if !capabilities
        .iter()
        .any(|capability| is_legacy_delegation_id(capability.id()))
    {
        return capabilities.to_vec();
    }

    let find = |id: &str| {
        capabilities
            .iter()
            .find(|capability| capability.id() == id)
            .map(CapabilityRef::config_value)
    };
    let subagents = find(LEGACY_SUBAGENTS_ID);
    let handoff = find(LEGACY_AGENT_HANDOFF_ID);
    let coordination = find(LEGACY_COORDINATION_ID)
        .filter(|config| config.get("role").and_then(Value::as_str) != Some("worker"));
    let has_new = |id: &str| capabilities.iter().any(|capability| capability.id() == id);

    let handoff_members = handoff.map(legacy_members).unwrap_or_default();
    let threads = coordination
        .filter(|_| !has_new(THREADS_CAPABILITY_ID))
        .map(|config| legacy_threads_config(config, &handoff_members));
    // A hand-off with no members and no subagents names nobody to spawn.
    let spawn = (coordination.is_none()
        && !has_new(SPAWN_AGENTS_CAPABILITY_ID)
        && !has_new(THREADS_CAPABILITY_ID)
        && (subagents.is_some() || !handoff_members.is_empty()))
    .then(|| legacy_spawn_config(subagents, &handoff_members));

    let mut out = Vec::with_capacity(capabilities.len() + 1);
    let mut spawn = spawn;
    let mut threads = threads;
    for capability in capabilities {
        match capability.id() {
            LEGACY_SUBAGENTS_ID | LEGACY_AGENT_HANDOFF_ID => {
                if let Some(config) = spawn.take() {
                    out.push(CapabilityRef::with_config(
                        SPAWN_AGENTS_CAPABILITY_ID,
                        config,
                    ));
                }
            }
            LEGACY_COORDINATION_ID => {
                if let Some(config) = threads.take() {
                    out.push(CapabilityRef::with_config(THREADS_CAPABILITY_ID, config));
                    if !has_new(COORDINATOR_CAPABILITY_ID) {
                        out.push(CapabilityRef::new(COORDINATOR_CAPABILITY_ID));
                    }
                }
            }
            LEGACY_RUN_CONVERSATION_ID => {
                if !has_new(SIBLING_COMMUNICATION_CAPABILITY_ID) {
                    out.push(CapabilityRef::with_config(
                        SIBLING_COMMUNICATION_CAPABILITY_ID,
                        capability.config_value().clone(),
                    ));
                }
            }
            _ => out.push(capability.clone()),
        }
    }
    // A coordinator now owns hand-off: spawn agents set alongside go away.
    if coordination.is_some() {
        out.retain(|capability| capability.id() != SPAWN_AGENTS_CAPABILITY_ID);
    }
    out
}

/// Hand-off members (`targets`, or `members`) as `agents` entries.
fn legacy_members(config: &Value) -> Vec<Value> {
    config
        .get("targets")
        .or_else(|| config.get("members"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|member| member.get("agent_id").is_some())
        .collect()
}

fn copy_key(from: Option<&Value>, keys: &[&str], to: &mut Map<String, Value>, key: &str) {
    if let Some(value) = from.and_then(|config| keys.iter().find_map(|k| config.get(*k))) {
        to.insert(key.to_string(), value.clone());
    }
}

fn legacy_spawn_config(subagents: Option<&Value>, members: &[Value]) -> Value {
    let mut config = Map::new();
    config.insert("self".to_string(), json!(subagents.is_some()));
    if !members.is_empty() {
        config.insert("agents".to_string(), Value::Array(members.to_vec()));
    }
    copy_key(
        subagents,
        &["max_subagent_depth", "max_depth"],
        &mut config,
        "max_depth",
    );
    copy_key(
        subagents,
        &[
            "max_active_descendant_tasks",
            "max_concurrent_descendant_tasks",
        ],
        &mut config,
        "max_open",
    );
    copy_key(
        subagents,
        &["max_total_descendant_tasks"],
        &mut config,
        "max_total",
    );
    copy_key(subagents, &["helper_models"], &mut config, "helper_models");
    copy_key(
        subagents,
        &["helper_model_pick"],
        &mut config,
        "helper_model_pick",
    );
    Value::Object(config)
}

fn legacy_threads_config(coordination: &Value, members: &[Value]) -> Value {
    let workers: Vec<String> = coordination
        .get("workers")
        .and_then(Value::as_array)
        .map(|workers| {
            workers
                .iter()
                .filter_map(|worker| worker.get("id").and_then(Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_else(|| vec!["self".to_string()]);
    let mut agents: Vec<Value> = members.to_vec();
    for id in workers.iter().filter(|id| *id != "self" && *id != "any") {
        if !agents
            .iter()
            .any(|agent| agent.get("agent_id").and_then(Value::as_str) == Some(id))
        {
            agents.push(json!({ "agent_id": id }));
        }
    }
    let mut config = Map::new();
    config.insert(
        "self".to_string(),
        json!(workers.iter().any(|id| id == "self")),
    );
    if workers.iter().any(|id| id == "any") {
        config.insert("any_agent".to_string(), json!(true));
    }
    if !agents.is_empty() {
        config.insert("agents".to_string(), Value::Array(agents));
    }
    copy_key(
        Some(coordination),
        &["max_active_threads"],
        &mut config,
        "max_open",
    );
    copy_key(
        Some(coordination),
        &["max_total_threads"],
        &mut config,
        "max_total",
    );
    Value::Object(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(capabilities: &[CapabilityRef]) -> Vec<&str> {
        capabilities.iter().map(CapabilityRef::id).collect()
    }

    #[test]
    fn current_ids_pass_through() {
        let input = vec![
            CapabilityRef::new("skills"),
            CapabilityRef::with_config(SPAWN_AGENTS_CAPABILITY_ID, json!({"self": true})),
        ];
        assert_eq!(canonicalize_delegation_capabilities(&input), input);
    }

    #[test]
    fn subagents_and_handoff_fold_into_one_spawn_agents() {
        let agent = "agent_01933b5a00007000800000000000a001";
        let input = vec![
            CapabilityRef::new("skills"),
            CapabilityRef::with_config("subagents", json!({"max_subagent_depth": 1})),
            CapabilityRef::with_config(
                "agent_handoff",
                json!({"targets": [{"agent_id": agent, "files": "copy"}]}),
            ),
        ];
        let out = canonicalize_delegation_capabilities(&input);
        assert_eq!(ids(&out), ["skills", "spawn_agents"]);
        assert_eq!(
            out[1].config_value(),
            &json!({"self": true, "max_depth": 1, "agents": [{"agent_id": agent, "files": "copy"}]})
        );
        assert_eq!(canonicalize_delegation_capabilities(&out), out);
    }

    #[test]
    fn handoff_alone_does_not_spawn_self() {
        let agent = "agent_01933b5a00007000800000000000a001";
        let out = canonicalize_delegation_capabilities(&[CapabilityRef::with_config(
            "agent_handoff",
            json!({"members": [{"agent_id": agent}]}),
        )]);
        assert_eq!(out[0].config_value()["self"], json!(false));
    }

    #[test]
    fn coordination_becomes_threads_and_coordinator_and_drops_spawn() {
        let agent = "agent_01933b5a00007000800000000000a001";
        let other = "agent_01933b5a00007000800000000000a002";
        let input = vec![
            CapabilityRef::new("subagents"),
            CapabilityRef::with_config("agent_handoff", json!({"targets": [{"agent_id": agent}]})),
            CapabilityRef::with_config(
                "coordination",
                json!({"workers": [{"id": "self"}, {"id": "any"}, {"id": other}], "max_active_threads": 3}),
            ),
            CapabilityRef::new("run_conversation"),
        ];
        let out = canonicalize_delegation_capabilities(&input);
        assert_eq!(
            ids(&out),
            ["threads", "coordinator", "sibling_communication"]
        );
        assert_eq!(
            out[0].config_value(),
            &json!({
                "self": true,
                "any_agent": true,
                "agents": [{"agent_id": agent}, {"agent_id": other}],
                "max_open": 3
            })
        );
    }

    #[test]
    fn worker_role_is_dropped() {
        let out = canonicalize_delegation_capabilities(&[
            CapabilityRef::new("subagents"),
            CapabilityRef::with_config("coordination", json!({"role": "worker"})),
        ]);
        assert_eq!(ids(&out), ["spawn_agents"]);
    }
}
