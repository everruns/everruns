//! Recorded tool changes, replayed from the transcript before each request.
//!
//! Like Astra effort snapshots (`reasoning_updates`), entries live on
//! completed assistant messages, so the immutable transcript is the record across worker restarts
//! and workers. An entry describes what changed before the generation that
//! produced its message: the first one of an epoch carries the baseline, later
//! ones the difference between the declared tools and the tools wanted then.
//!
//! An epoch ends wherever the cached prefix restarts anyway: a message from
//! another model, a transcript cut by a compaction checkpoint (its stored
//! messages are gone, so no baseline is found), or a session from before this
//! existed. The next request then records a new baseline.
//!
//! Drivers that render changes in a cache-safe form send the epoch's baseline
//! as their tool list and the transitions inline; the others ignore both, since
//! `tools` still carries the wanted set. The baseline is offered only when every
//! change of the epoch reaches the request (see [`ToolChangeReplay::baseline_for`]).
//!
//! The system prompt is tracked the same way: a baseline records it, and parts
//! a capability adds later become the change's `note`, so inline requests keep
//! the epoch's prompt. When a recorded part changes or disappears, no prompt
//! baseline is offered and drivers send the current prompt.
//!
//! On models that cannot carry changes inline, every change rewrites `tools`
//! and costs a cache write. A removal is the one change nothing needs at once:
//! while the cache is warm and nothing else changes, it is held. The removed
//! tool stays in `tools` and is not recorded as removed, and a call to it is
//! refused like any undeclared tool, because the act step only runs the
//! agent's current tools. It goes out with the next change that rewrites
//! `tools` anyway, or once the session has sat idle past the cache lifetime.

// Design of record: knowledge/execution/cache-stable-tool-changes.md.
use crate::engine::message::{RuntimeMessage, RuntimeMessageRole};
use crate::engine::tool_types::ToolDefinition;
use crate::engine::typed_id::MessageId;
use everruns_contracts::driver_registry::Message;
use everruns_contracts::tool_changes::{
    DeclaredPrompt, DeclaredTools, PromptDiff, ToolChanges, prompt_note,
};
use std::collections::HashMap;

pub(super) const KEY: &str = everruns_contracts::tool_changes::METADATA_KEY;

#[derive(Debug, Default)]
pub(super) struct ToolChangeReplay {
    /// Changes keyed by the message they take effect immediately before.
    pub transitions: HashMap<MessageId, ToolChanges>,
    /// Entry to store on this generation's assistant message, if any.
    pub record: Option<ToolChanges>,
    /// Tools fixed at the start of the epoch.
    baseline: Option<Vec<ToolDefinition>>,
    /// System prompt fixed at the start of the epoch, when everything added to
    /// it since is in the recorded notes.
    system_prompt: Option<String>,
    /// This request's change has no message to ride on.
    unanchored: bool,
    /// The tool list to send instead of the wanted one, when a removal is
    /// held: the wanted tools with the held ones kept in their places.
    held_tools: Option<Vec<ToolDefinition>>,
    /// The wanted tools in declaration order: tools sent before keep their
    /// places and new ones go last. Models that fold changes into `tools`
    /// then keep the cached prefix up to the first new tool instead of losing
    /// it wherever a revealed tool sorts in.
    ordered_tools: Option<Vec<ToolDefinition>>,
}

/// When a removal may wait: the request time and how long the provider keeps
/// a cached prefix after its last use.
#[derive(Debug, Clone, Copy)]
pub(super) struct ColdMoment {
    pub now: chrono::DateTime<chrono::Utc>,
    pub cache_lifetime: chrono::Duration,
}

impl ColdMoment {
    /// For models that rewrite `tools` on every change; `None` for those that
    /// carry removals inline, where a removal costs nothing.
    fn for_model(
        provider: &everruns_contracts::DriverId,
        model: &str,
        cache: Option<&everruns_contracts::driver_registry::PromptCacheConfig>,
    ) -> Option<Self> {
        use everruns_contracts::DriverId;
        use everruns_contracts::driver_registry::PromptCacheRetention;
        use everruns_contracts::model::{
            INLINE_TOOL_ADDITIONS_PARAMETER, INLINE_TOOL_CHANGES_PARAMETER,
        };
        // Only the first-party Claude and OpenAI drivers send changes inline;
        // a gateway serving the same model resolves to the same profile.
        let first_party = *provider == DriverId::Anthropic || *provider == DriverId::OpenAI;
        let inline = first_party
            && everruns_contracts::get_model_profile(provider, model).is_some_and(|p| {
                p.supports_parameter(INLINE_TOOL_CHANGES_PARAMETER)
                    || p.supports_parameter(INLINE_TOOL_ADDITIONS_PARAMETER)
            });
        if inline {
            return None;
        }
        // Overstating the lifetime only holds a removal longer; understating
        // it would release one into a warm cache.
        let retention = cache.map(|cache| cache.retention).unwrap_or_default();
        let minutes = match retention {
            PromptCacheRetention::OneHour => 60,
            _ if *provider == DriverId::Anthropic || *provider == DriverId::Bedrock => 5,
            _ => 30,
        };
        Some(Self {
            now: chrono::Utc::now(),
            cache_lifetime: chrono::Duration::minutes(minutes),
        })
    }

    fn warm_since(&self, last_use: Option<chrono::DateTime<chrono::Utc>>) -> bool {
        last_use.is_some_and(|at| self.now - at < self.cache_lifetime)
    }
}

impl ToolChangeReplay {
    /// Store the entry on the assistant message's metadata.
    pub(super) fn stamp(&self, metadata: &mut HashMap<String, serde_json::Value>) {
        if let Some(record) = &self.record {
            metadata.insert(KEY.to_string(), serde_json::json!(record));
        }
    }

    /// The epoch's baseline, when every change since it is on a message of
    /// `sent`. A change dropped from the request (a model view or trimming cut
    /// its message) would leave the model unaware of a tool, so drivers then
    /// get no baseline and send the wanted set as before.
    pub(super) fn baseline_for(&self, sent: &[Message]) -> Option<Vec<ToolDefinition>> {
        self.complete(sent).then(|| self.baseline.clone()).flatten()
    }

    /// The epoch's system prompt, offered on the same condition as
    /// [`Self::baseline_for`]: a note dropped from the request would leave the
    /// model without the text it carries.
    pub(super) fn system_prompt_for(&self, sent: &[Message]) -> Option<String> {
        self.complete(sent)
            .then(|| self.system_prompt.clone())
            .flatten()
    }

    /// Offer the epoch's tools and system prompt to the drivers that send
    /// changes inline.
    pub(super) fn apply_to(
        &self,
        config: &mut everruns_contracts::driver_registry::LlmCallConfig,
        sent: &[Message],
    ) {
        config.tool_baseline = self.baseline_for(sent);
        config.system_prompt_baseline = self.system_prompt_for(sent);
        if let Some(tools) = self.held_tools.as_ref().or(self.ordered_tools.as_ref()) {
            config.tools.clone_from(tools);
        }
    }

    fn complete(&self, sent: &[Message]) -> bool {
        let carried = sent.iter().filter(|m| m.tool_changes.is_some()).count();
        !self.unanchored && carried == self.transitions.len()
    }
}

fn entry(message: &RuntimeMessage) -> Option<ToolChanges> {
    let value = message.metadata.as_ref()?.get(KEY)?;
    serde_json::from_value(value.clone()).ok()
}

fn same_model(message: &RuntimeMessage, provider: &str, model: &str) -> bool {
    let field = |key| {
        message
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.get(key))
            .and_then(serde_json::Value::as_str)
    };
    field("provider") == Some(provider) && field("model") == Some(model)
}

/// The first non-system message from `start` through `end`: where a change
/// recorded on the assistant message at `end` took effect.
fn anchor(messages: &[&RuntimeMessage], start: usize, end: usize) -> Option<MessageId> {
    messages
        .get(start..=end)?
        .iter()
        .find(|message| message.role != RuntimeMessageRole::System)
        .map(|message| message.id)
}

/// The wanted tools with each held one put back after the declared tool it
/// followed, so the list matches the one sent before the removal.
fn with_held(
    declared: &DeclaredTools,
    wanted: &[ToolDefinition],
    held: &[String],
) -> Vec<ToolDefinition> {
    let mut tools = wanted.to_vec();
    let mut after: Option<&str> = None;
    for tool in declared.callable() {
        let name = tool.name();
        if held.iter().any(|held| held == name) {
            let at = after
                .and_then(|after| tools.iter().position(|t| t.name() == after))
                .map_or(0, |index| index + 1);
            tools.insert(at, tool.clone());
        } else if !wanted.iter().any(|t| t.name() == name) {
            continue;
        }
        after = Some(name);
    }
    tools
}

/// [`prepare_holding`] for the agent's current tools and system prompt.
pub(super) fn prepare_for(
    messages: &[RuntimeMessage],
    provider: &everruns_contracts::DriverId,
    model: &str,
    agent: &crate::engine::RuntimeAgent,
) -> ToolChangeReplay {
    let replay = prepare_holding(
        messages,
        provider.as_str(),
        model,
        &agent.tools,
        &agent.system_prompt,
        ColdMoment::for_model(provider, model, agent.prompt_cache.as_ref()),
    );
    // A live reduction of the disclosure budget takes effect immediately.
    // Ordinary removal-only changes still preserve a warm provider cache.
    if replay.held_tools.as_ref().is_some_and(|held| {
        agent
            .model_tool_budget
            .is_some_and(|budget| !budget.admits(held))
    }) {
        return prepare_holding(
            messages,
            provider.as_str(),
            model,
            &agent.tools,
            &agent.system_prompt,
            None,
        );
    }
    replay
}

#[cfg(test)]
pub(super) fn prepare(
    messages: &[RuntimeMessage],
    provider: &str,
    model: &str,
    wanted: &[ToolDefinition],
    system_prompt: &str,
) -> ToolChangeReplay {
    prepare_holding(messages, provider, model, wanted, system_prompt, None)
}

/// Replay the transcript and work out this request's change. With `cold`,
/// a change that only removes tools waits while the cache is warm.
pub(super) fn prepare_holding(
    messages: &[RuntimeMessage],
    provider: &str,
    model: &str,
    wanted: &[ToolDefinition],
    system_prompt: &str,
    cold: Option<ColdMoment>,
) -> ToolChangeReplay {
    let messages: Vec<_> = messages
        .iter()
        .filter(|message| !super::is_error_placeholder_message(message))
        .collect();
    let mut declared: Option<DeclaredTools> = None;
    let mut prompt = DeclaredPrompt::default();
    let mut baseline = None;
    let mut transitions = HashMap::new();
    let mut input_start = 0;
    let mut last_answer = None;
    for (index, message) in messages.iter().enumerate() {
        if message.role != RuntimeMessageRole::Agent {
            continue;
        }
        last_answer = Some(message.created_at);
        if !same_model(message, provider, model) {
            declared = None;
            transitions.clear();
        } else if let Some(changes) = entry(message) {
            if changes.baseline.is_some() {
                declared = Some(DeclaredTools::default());
                baseline.clone_from(&changes.baseline);
                transitions.clear();
            }
            if let Some(declared) = declared.as_mut() {
                declared.apply(&changes);
                prompt.apply(&changes);
                if !changes.is_empty()
                    && let Some(at) = anchor(&messages, input_start, index)
                {
                    transitions.insert(at, changes);
                }
            }
        }
        input_start = index + 1;
    }

    let Some(declared) = declared else {
        return ToolChangeReplay {
            record: Some(ToolChanges::baseline_with_prompt(
                wanted.to_vec(),
                system_prompt,
            )),
            baseline: Some(wanted.to_vec()),
            system_prompt: Some(system_prompt.to_string()),
            ..ToolChangeReplay::default()
        };
    };
    let mut changes = declared.diff(wanted);
    let system_prompt = match prompt.diff(system_prompt) {
        PromptDiff::Added(parts) => {
            changes.note = prompt_note(&parts);
            prompt.baseline().map(str::to_string)
        }
        PromptDiff::Diverged => None,
    };
    let only_removes = changes.added.is_empty()
        && changes.redefined.is_empty()
        && changes.note.is_none()
        && !changes.removed.is_empty();
    let held_tools = (only_removes && cold.is_some_and(|cold| cold.warm_since(last_answer)))
        .then(|| with_held(&declared, wanted, &std::mem::take(&mut changes.removed)));
    let ordered_tools = Some(in_declared_order(&declared, &changes));
    if changes.is_empty() {
        return ToolChangeReplay {
            transitions,
            baseline,
            system_prompt,
            held_tools,
            ordered_tools,
            ..ToolChangeReplay::default()
        };
    }
    // With no new input since the last answer, the change precedes the output
    // being generated now; its replay anchors it there next time.
    let at = anchor(&messages, input_start, messages.len().saturating_sub(1));
    if let Some(at) = at {
        transitions.insert(at, changes.clone());
    }
    ToolChangeReplay {
        transitions,
        record: Some(changes),
        baseline,
        system_prompt,
        unanchored: at.is_none(),
        held_tools: None,
        ordered_tools,
    }
}

/// The declaration after `changes`: the wanted set, with tools already
/// declared in their places and added ones after them in the order added.
fn in_declared_order(declared: &DeclaredTools, changes: &ToolChanges) -> Vec<ToolDefinition> {
    let mut declared = declared.clone();
    declared.apply(changes);
    declared.callable().cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const PROVIDER: &str = "anthropic";
    const MODEL: &str = "claude-sonnet-5-5";
    const PROMPT: &str = "base";

    fn tool(name: &str, description: &str) -> ToolDefinition {
        ToolDefinition::function(name, description, json!({"type": "object"}))
    }

    fn answer(model: &str, changes: Option<&ToolChanges>) -> RuntimeMessage {
        let mut message = RuntimeMessage::assistant("done");
        let mut metadata = HashMap::from([
            ("provider".to_string(), json!(PROVIDER)),
            ("model".to_string(), json!(model)),
        ]);
        if let Some(changes) = changes {
            metadata.insert(KEY.to_string(), json!(changes));
        }
        message.metadata = Some(metadata);
        message
    }

    /// Runs `prepare` and appends the answer it would store.
    fn step(
        messages: &mut Vec<RuntimeMessage>,
        input: &str,
        wanted: &[ToolDefinition],
    ) -> ToolChangeReplay {
        messages.push(RuntimeMessage::user(input));
        let replay = prepare(messages, PROVIDER, MODEL, wanted, PROMPT);
        messages.push(answer(MODEL, replay.record.as_ref()));
        replay
    }

    #[test]
    fn the_first_request_records_a_baseline_and_nothing_else() {
        let mut messages = Vec::new();
        let replay = step(&mut messages, "hi", &[tool("a", "A")]);
        let record = replay.record.expect("baseline recorded");
        assert_eq!(record.baseline.as_ref().map(Vec::len), Some(1));
        assert!(record.is_empty());
        assert!(replay.transitions.is_empty());

        let replay = step(&mut messages, "again", &[tool("a", "A")]);
        assert!(replay.record.is_none());
        assert!(replay.transitions.is_empty());
    }

    #[test]
    fn a_change_is_recorded_once_and_replayed_at_its_input() {
        let mut messages = Vec::new();
        step(&mut messages, "hi", &[tool("a", "A")]);
        let wanted = [tool("a", "A"), tool("b", "B")];
        let replay = step(&mut messages, "reveal b", &wanted);
        let input = messages[2].id;
        let record = replay.record.expect("change recorded");
        assert!(record.baseline.is_none());
        assert_eq!(record.added[0].name(), "b");
        assert_eq!(replay.transitions[&input].added[0].name(), "b");

        // Later requests replay the stored change at the same input and
        // record nothing new.
        let replay = step(&mut messages, "next", &wanted);
        assert!(replay.record.is_none());
        assert_eq!(replay.transitions.len(), 1);
        assert_eq!(replay.transitions[&input].added[0].name(), "b");
    }

    /// Tools already sent keep their places and a revealed tool goes last, even
    /// when the wanted list sorts it first, so folded `tools` only grow at the
    /// end and the cached prefix up to them survives.
    #[test]
    fn revealed_tools_are_sent_after_the_ones_already_sent() {
        use everruns_contracts::driver_registry::LlmCallConfig;
        let order = |replay: &ToolChangeReplay, wanted: &[ToolDefinition]| {
            let mut config = LlmCallConfig::new(MODEL);
            config.tools = wanted.to_vec();
            replay.apply_to(&mut config, &[]);
            config
                .tools
                .iter()
                .map(|t| t.name().to_string())
                .collect::<Vec<_>>()
        };
        let mut messages = Vec::new();
        step(&mut messages, "hi", &[tool("b", "B"), tool("d", "D")]);
        let wanted = [
            tool("a", "A"),
            tool("b", "B"),
            tool("c", "C"),
            tool("d", "D"),
        ];
        let replay = step(&mut messages, "reveal a and c", &wanted);
        assert_eq!(order(&replay, &wanted), ["b", "d", "a", "c"]);

        // The next request sends the same list.
        let replay = step(&mut messages, "next", &wanted);
        assert_eq!(order(&replay, &wanted), ["b", "d", "a", "c"]);

        // A removal together with a reveal drops the tool in place.
        let wanted = [
            tool("a", "A"),
            tool("c", "C"),
            tool("d", "D"),
            tool("e", "E"),
        ];
        let replay = step(&mut messages, "swap", &wanted);
        assert_eq!(order(&replay, &wanted), ["d", "a", "c", "e"]);
    }

    #[test]
    fn a_removed_tool_is_announced_once() {
        let mut messages = Vec::new();
        step(&mut messages, "hi", &[tool("a", "A"), tool("b", "B")]);
        let replay = step(&mut messages, "drop b", &[tool("a", "A")]);
        assert_eq!(replay.record.expect("removal").removed, ["b"]);
        assert!(
            step(&mut messages, "next", &[tool("a", "A")])
                .record
                .is_none()
        );
    }

    #[test]
    fn another_model_starts_a_new_epoch() {
        let mut messages = Vec::new();
        step(&mut messages, "hi", &[tool("a", "A")]);
        step(&mut messages, "reveal", &[tool("a", "A"), tool("b", "B")]);
        messages.push(RuntimeMessage::user("switch"));
        messages.push(answer("claude-opus-5-5", None));

        let replay = step(&mut messages, "back", &[tool("a", "A"), tool("b", "B")]);
        let record = replay.record.expect("new baseline");
        assert_eq!(record.baseline.as_ref().map(Vec::len), Some(2));
        assert!(replay.transitions.is_empty());
    }

    #[test]
    fn the_baseline_is_offered_only_when_every_change_is_sent() {
        use everruns_contracts::driver_registry::MessageRole;
        let mut messages = Vec::new();
        step(&mut messages, "hi", &[tool("a", "A")]);
        let wanted = [tool("a", "A"), tool("b", "B")];
        let replay = step(&mut messages, "reveal b", &wanted);
        let sent = |with_change: bool| {
            let mut input = Message::text(MessageRole::User, "reveal b");
            input.tool_changes = with_change.then(|| replay.transitions[&messages[2].id].clone());
            vec![Message::text(MessageRole::User, "hi"), input]
        };
        let baseline = replay.baseline_for(&sent(true)).expect("all changes sent");
        assert_eq!(baseline.len(), 1, "the epoch's first tool list");
        assert!(replay.baseline_for(&sent(false)).is_none());

        // A change with no message to ride on (no input since the last
        // answer) cannot be sent inline either.
        let replay = prepare(&messages, PROVIDER, MODEL, &[tool("a", "A")], PROMPT);
        assert!(replay.record.is_some());
        assert!(replay.baseline_for(&sent(true)).is_none());
    }

    #[test]
    fn a_transcript_without_entries_starts_with_a_baseline() {
        let mut messages = vec![RuntimeMessage::user("old"), answer(MODEL, None)];
        let replay = step(&mut messages, "new", &[tool("a", "A")]);
        assert!(replay.record.expect("baseline").baseline.is_some());
    }

    #[test]
    fn prompt_text_added_mid_session_rides_in_a_note() {
        use everruns_contracts::driver_registry::MessageRole;
        let tools = [tool("a", "A")];
        let mut messages = vec![RuntimeMessage::user("hi")];
        let replay = prepare(&messages, PROVIDER, MODEL, &tools, "base");
        assert_eq!(
            replay.record.as_ref().unwrap().system_prompt.as_deref(),
            Some("base")
        );
        messages.push(answer(MODEL, replay.record.as_ref()));

        // A capability switched on adds its how-to to the prompt.
        messages.push(RuntimeMessage::user("deploy"));
        let replay = prepare(&messages, PROVIDER, MODEL, &tools, "base\n\ndeploy how-to");
        let record = replay.record.clone().expect("note recorded");
        assert_eq!(record.note.as_deref(), Some("deploy how-to"));
        let mut input = Message::text(MessageRole::User, "deploy");
        input.tool_changes = Some(record.clone());
        let sent = vec![Message::text(MessageRole::User, "hi"), input];
        assert_eq!(replay.system_prompt_for(&sent).as_deref(), Some("base"));
        messages.push(answer(MODEL, Some(&record)));

        // Recorded once: the next request adds nothing and keeps the prompt.
        messages.push(RuntimeMessage::user("next"));
        let replay = prepare(&messages, PROVIDER, MODEL, &tools, "base\n\ndeploy how-to");
        assert!(replay.record.is_none());
        assert_eq!(replay.system_prompt_for(&sent).as_deref(), Some("base"));

        // A part that changes cannot ride in a note: send the current prompt.
        let replay = prepare(
            &messages,
            PROVIDER,
            MODEL,
            &tools,
            "base v2\n\ndeploy how-to",
        );
        assert!(replay.system_prompt_for(&sent).is_none());
        assert!(
            replay.baseline_for(&sent).is_some(),
            "tools stay cache-safe"
        );
    }

    fn after(minutes: i64) -> Option<ColdMoment> {
        Some(ColdMoment {
            now: chrono::Utc::now() + chrono::Duration::minutes(minutes),
            cache_lifetime: chrono::Duration::minutes(5),
        })
    }

    fn holding(
        messages: &mut Vec<RuntimeMessage>,
        input: &str,
        wanted: &[ToolDefinition],
        cold: Option<ColdMoment>,
    ) -> ToolChangeReplay {
        messages.push(RuntimeMessage::user(input));
        let replay = prepare_holding(messages, PROVIDER, MODEL, wanted, PROMPT, cold);
        messages.push(answer(MODEL, replay.record.as_ref()));
        replay
    }

    fn names(tools: &[ToolDefinition]) -> Vec<&str> {
        tools.iter().map(ToolDefinition::name).collect()
    }

    #[test]
    fn a_removal_waits_while_the_cache_is_warm() {
        let all = [tool("a", "A"), tool("b", "B"), tool("c", "C")];
        let mut messages = Vec::new();
        holding(&mut messages, "hi", &all, after(1));
        let wanted = [tool("a", "A"), tool("c", "C")];

        let replay = holding(&mut messages, "drop b", &wanted, after(1));
        assert!(replay.record.is_none(), "nothing recorded");
        assert!(replay.transitions.is_empty());
        let mut config = everruns_contracts::driver_registry::LlmCallConfig::new(MODEL);
        config.tools = wanted.to_vec();
        replay.apply_to(&mut config, &[]);
        assert_eq!(names(&config.tools), ["a", "b", "c"], "b kept in place");

        // Still warm: still held, nothing recorded.
        let replay = holding(&mut messages, "next", &wanted, after(2));
        assert!(replay.record.is_none());
        assert!(replay.held_tools.is_some());

        // Past the cache lifetime the removal goes out.
        let replay = holding(&mut messages, "later", &wanted, after(10));
        assert_eq!(replay.record.expect("released").removed, ["b"]);
        assert!(replay.held_tools.is_none());
    }

    #[test]
    fn a_held_removal_rides_with_the_next_change() {
        let mut messages = Vec::new();
        holding(
            &mut messages,
            "hi",
            &[tool("a", "A"), tool("b", "B")],
            after(1),
        );
        holding(&mut messages, "drop b", &[tool("a", "A")], after(1));

        let wanted = [tool("a", "A"), tool("d", "D")];
        let replay = holding(&mut messages, "reveal d", &wanted, after(1));
        let record = replay.record.expect("one change");
        assert_eq!(names(&record.added), ["d"]);
        assert_eq!(record.removed, ["b"]);
        assert!(replay.held_tools.is_none(), "tools are rewritten anyway");
    }

    #[test]
    fn models_with_inline_changes_never_hold() {
        use everruns_contracts::DriverId;
        use everruns_contracts::driver_registry::{PromptCacheConfig, PromptCacheRetention};
        assert!(ColdMoment::for_model(&DriverId::Anthropic, "claude-sonnet-5-5", None).is_none());
        assert!(ColdMoment::for_model(&DriverId::OpenAI, "gpt-6.1-sol", None).is_none());
        assert!(
            ColdMoment::for_model(&DriverId::OpenRouter, "anthropic/claude-sonnet-5-5", None)
                .is_some(),
            "a gateway sends the full tool list even for a model with inline changes"
        );

        let fallback = ColdMoment::for_model(&DriverId::Gemini, "gemini-unknown", None)
            .expect("fallback model holds");
        assert_eq!(fallback.cache_lifetime, chrono::Duration::minutes(30));
        let hour = PromptCacheConfig {
            retention: PromptCacheRetention::OneHour,
            ..PromptCacheConfig::default()
        };
        let held = ColdMoment::for_model(&DriverId::Gemini, "gemini-unknown", Some(&hour))
            .expect("fallback model holds");
        assert_eq!(held.cache_lifetime, chrono::Duration::minutes(60));
    }

    #[test]
    fn live_budget_reduction_releases_a_fallback_hold_but_ordinary_removal_keeps_it() {
        use everruns_contracts::runtime::capabilities::ModelToolBudget;
        let provider = everruns_contracts::DriverId::Gemini;
        let model = "gemini-unknown";
        let all = vec![tool("a", "A"), tool("b", "B")];
        let mut agent = crate::engine::RuntimeAgent::new(PROMPT, model);
        agent.tools = all.clone();
        let initial = prepare_for(&[RuntimeMessage::user("first")], &provider, model, &agent);
        let mut answer = RuntimeMessage::assistant("done");
        answer.metadata = Some(HashMap::from([
            ("provider".into(), json!(provider.as_str())),
            ("model".into(), json!(model)),
        ]));
        initial.stamp(answer.metadata.as_mut().unwrap());
        let messages = vec![
            RuntimeMessage::user("first"),
            answer,
            RuntimeMessage::user("remove b"),
        ];
        agent.tools.pop();
        agent.model_tool_budget = Some(ModelToolBudget {
            max_tools: 2,
            max_bytes: 4096,
        });
        assert!(
            prepare_for(&messages, &provider, model, &agent)
                .held_tools
                .is_some()
        );
        for budget in [
            ModelToolBudget {
                max_tools: 1,
                max_bytes: 4096,
            },
            ModelToolBudget {
                max_tools: 2,
                max_bytes: serde_json::to_vec(&agent.tools).unwrap().len(),
            },
        ] {
            agent.model_tool_budget = Some(budget);
            let replay = prepare_for(&messages, &provider, model, &agent);
            assert!(replay.held_tools.is_none());
            assert_eq!(replay.record.unwrap().removed, ["b"]);
        }
    }
}
