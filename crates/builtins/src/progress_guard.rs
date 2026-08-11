// Progress guard for coding-agent efficiency.
//
// This is intentionally runtime-enforced rather than prompt-only: it observes
// tool traffic and injects a warning into the next tool result when the turn is
// spending many tools on investigation without edits or validation.

use crate::atoms::{PostToolExecHook, PostToolExecHookPriority};
use crate::capabilities::{Capability, CapabilityStatus};
use crate::tool_types::{ToolCall, ToolDefinition, ToolResult};
use crate::traits::ToolContext;
use async_trait::async_trait;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet, VecDeque, hash_map::DefaultHasher};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};

pub const PROGRESS_GUARD_CAPABILITY_ID: &str = "progress_guard";

const EXPLORATION_WITHOUT_PROGRESS_THRESHOLD: usize = 24;
const CHECKPOINT_WITHOUT_PROGRESS_THRESHOLD: usize = 48;
const REPEATED_EXPLORATION_THRESHOLD: usize = 5;
const ZERO_EVIDENCE_SEARCH_THRESHOLD: usize = 3;
const TRUNCATED_EXPLORATION_THRESHOLD: usize = 2;
const REPEATED_STATUS_THRESHOLD: usize = 3;
const WAITING_WINDOW_SIZE: usize = 8;
const WAITING_WINDOW_THRESHOLD: usize = 4;
const SEMANTIC_HISTORY_LIMIT: usize = 512;
const TRACKED_PATH_LIMIT: usize = 1024;
const MIN_REUSABLE_RESULT_BYTES: usize = 512;

pub struct ProgressGuardCapability {
    state: Arc<Mutex<ProgressGuardState>>,
}

impl ProgressGuardCapability {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(ProgressGuardState::default())),
        }
    }
}

impl Default for ProgressGuardCapability {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Capability for ProgressGuardCapability {
    fn id(&self) -> &str {
        PROGRESS_GUARD_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Progress Guard"
    }

    fn description(&self) -> &str {
        "Warns the coding agent when tool usage suggests investigation without progress."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn category(&self) -> Option<&str> {
        Some("Guardrails")
    }

    fn is_guardrail(&self) -> bool {
        true
    }

    // No system-prompt contribution. Every warning this capability emits already
    // names the situation and the required next action, and it arrives in the
    // tool result at the moment it applies. Pre-announcing the mechanism on every
    // turn paid for a warning that usually never fires.

    fn system_prompt_preview(&self) -> Option<String> {
        Some(
            "<capability id=\"progress_guard\">\nWarns on investigation without progress.\n</capability>"
                .to_string(),
        )
    }

    fn post_tool_exec_hooks(&self) -> Vec<Arc<dyn PostToolExecHook>> {
        vec![Arc::new(ProgressGuardHook {
            state: self.state.clone(),
        })]
    }
}

#[derive(Default)]
struct ProgressGuardState {
    sessions: HashMap<String, SessionProgress>,
}

#[derive(Default)]
struct SessionProgress {
    tool_count: usize,
    exploration_since_progress: usize,
    mutation_count: usize,
    validation_count: usize,
    repeated_status_count: usize,
    last_status_command: Option<String>,
    recent_waiting: VecDeque<bool>,
    repeated_exploration_count: usize,
    last_exploration_signature: Option<String>,
    consecutive_zero_evidence_searches: usize,
    consecutive_truncated_exploration: usize,
    warning_count: usize,
    workspace_epoch: u64,
    workspace_hashes: HashMap<String, String>,
    recent_workspace_states: VecDeque<u64>,
    seen_workspace_states: HashSet<u64>,
    recent_validations: VecDeque<(u64, String)>,
    seen_validations: HashSet<(u64, String)>,
    recent_observations: VecDeque<String>,
    observation_hashes: HashMap<String, [u8; 32]>,
}

impl SessionProgress {
    fn observe(&mut self, tool_call: &ToolCall, result: &mut ToolResult) -> Option<String> {
        self.tool_count += 1;
        let class = classify_tool_call(tool_call);

        match class {
            ToolClass::Mutation => self.observe_mutation(tool_call, result),
            ToolClass::Validation(command) => {
                self.validation_count += 1;
                self.reset_activity_streaks();
                let validation = (self.validation_state_signature(), command);
                if self.seen_validations.contains(&validation) {
                    self.warning_count += 1;
                    return Some(
                        "progress_guard: repeated the same validation command on an unchanged workspace state. The result adds no new code evidence; use the existing result, change the relevant state, or explain why an external retry is necessary before running it again."
                            .to_string(),
                    );
                }
                remember_bounded(
                    &mut self.seen_validations,
                    &mut self.recent_validations,
                    validation,
                );
                None
            }
            ToolClass::Waiting => {
                self.exploration_since_progress += 1;
                self.reset_result_streaks();
                if self.observe_waiting_signal(true) {
                    self.warning_count += 1;
                    return Some(
                        "progress_guard: repeated checks of an external event (CI run, PR checks/reviews) without other progress. Do not poll across turns: run one blocking watch detached via spawn_background (e.g. `gh pr checks --watch` or an `until <check>; do sleep 30; done` loop) and end the turn — completion wakes the agent. In one-shot mode, block on the spawned task with wait_task instead."
                            .to_string(),
                    );
                }
                self.exploration_warning()
            }
            ToolClass::Status(command) => {
                self.exploration_since_progress += 1;
                self.reset_result_streaks();
                self.observe_waiting_signal(false);
                if self.last_status_command.as_deref() == Some(command.as_str()) {
                    self.repeated_status_count += 1;
                } else {
                    self.repeated_status_count = 1;
                    self.last_status_command = Some(command);
                }
                if self.repeated_status_count >= REPEATED_STATUS_THRESHOLD {
                    self.warning_count += 1;
                    self.repeated_status_count = 0;
                    return Some(
                        "progress_guard: repeated git status/diff checks without an intervening edit or validation. Use the latest result, make a targeted change, run a decisive check, or explain why no change is needed."
                            .to_string(),
                    );
                }
                self.exploration_warning()
            }
            ToolClass::Exploration => {
                self.exploration_since_progress += 1;
                self.observe_waiting_signal(false);
                if let Some(warning) = self.result_warning(tool_call, result) {
                    return Some(warning);
                }
                if let Some(warning) = self.reuse_unchanged_observation(tool_call, result) {
                    return Some(warning);
                }
                if let Some(warning) = self.repetition_warning(tool_call) {
                    return Some(warning);
                }
                self.exploration_warning()
            }
            ToolClass::Other => {
                self.observe_waiting_signal(false);
                self.reset_result_streaks();
                None
            }
        }
    }

    fn observe_mutation(&mut self, tool_call: &ToolCall, result: &ToolResult) -> Option<String> {
        if result.error.is_some() || !mutation_was_applied(tool_call, result) {
            return None;
        }

        self.mutation_count += 1;
        self.reset_activity_streaks();
        // A mutation changes the meaning of later repository evidence even when
        // a file happens to return to the same bytes. Clear reuse state instead
        // of serving a compact marker across an edit boundary.
        self.recent_observations.clear();
        self.observation_hashes.clear();

        let Some(transition) = mutation_hash_transition(tool_call, result) else {
            // Shell, delete, and structural edits can touch an unknown set of
            // files. They make validation fresh, but retaining structured file
            // hashes lets us still recognize a manifest cycle around lockfile
            // updates and other adjacent mutations.
            self.advance_opaque_mutation();
            return None;
        };

        if self.workspace_hashes.len() >= TRACKED_PATH_LIMIT
            && !self.workspace_hashes.contains_key(&transition.path)
        {
            self.reset_tracked_history();
        }

        if let Some(previous_hash) = transition.previous_hash {
            if let Some(known_hash) = self.workspace_hashes.get(&transition.path)
                && known_hash != &previous_hash
            {
                // An unobserved writer changed the file. Preserve safety by
                // abandoning comparisons with the now-incomplete trajectory.
                self.reset_tracked_history();
            }
            if !self.workspace_hashes.contains_key(&transition.path) {
                self.workspace_hashes
                    .insert(transition.path.clone(), previous_hash);
                let previous_state = self.tracked_state_signature();
                self.remember_workspace_state(previous_state);
            }
        }

        self.workspace_hashes
            .insert(transition.path, transition.content_hash);
        let current_state = self.tracked_state_signature();
        if self.remember_workspace_state(current_state) {
            self.warning_count += 1;
            return Some(
                "progress_guard: this mutation returned to a recently seen workspace state (the same tracked content hashes). Confirm the revert is intentional; if this is an edit/validate cycle, keep the coherent state, report the blocker, and stop repeating the cycle."
                    .to_string(),
            );
        }
        None
    }

    fn reset_activity_streaks(&mut self) {
        self.exploration_since_progress = 0;
        self.repeated_status_count = 0;
        self.last_status_command = None;
        self.recent_waiting.clear();
        self.repeated_exploration_count = 0;
        self.last_exploration_signature = None;
        self.reset_result_streaks();
    }

    fn observe_waiting_signal(&mut self, waiting: bool) -> bool {
        // A bounded semantic window catches heterogeneous check/read/task-probe
        // cycles while mutations and validations still clear it. Transparently
        // moving an already-started shell process would risk replaying side
        // effects and weakening one-shot cancellation, so the guard steers the
        // next wait onto the existing durable background-task path instead.
        self.recent_waiting.push_back(waiting);
        if self.recent_waiting.len() > WAITING_WINDOW_SIZE {
            self.recent_waiting.pop_front();
        }
        if self
            .recent_waiting
            .iter()
            .filter(|waiting| **waiting)
            .count()
            >= WAITING_WINDOW_THRESHOLD
        {
            self.recent_waiting.clear();
            return true;
        }
        false
    }

    fn advance_opaque_mutation(&mut self) {
        self.workspace_epoch = self.workspace_epoch.wrapping_add(1);
        self.recent_validations.clear();
        self.seen_validations.clear();
    }

    fn reset_tracked_history(&mut self) {
        self.workspace_hashes.clear();
        self.recent_workspace_states.clear();
        self.seen_workspace_states.clear();
        self.advance_opaque_mutation();
    }

    fn tracked_state_signature(&self) -> u64 {
        let mut entries = self.workspace_hashes.iter().collect::<Vec<_>>();
        entries.sort_unstable_by(|left, right| left.0.cmp(right.0));
        let mut hasher = DefaultHasher::new();
        entries.hash(&mut hasher);
        hasher.finish()
    }

    fn validation_state_signature(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.workspace_epoch.hash(&mut hasher);
        self.tracked_state_signature().hash(&mut hasher);
        hasher.finish()
    }

    fn remember_workspace_state(&mut self, state: u64) -> bool {
        if self.seen_workspace_states.contains(&state) {
            return true;
        }
        remember_bounded(
            &mut self.seen_workspace_states,
            &mut self.recent_workspace_states,
            state,
        );
        false
    }

    fn result_warning(&mut self, tool_call: &ToolCall, result: &ToolResult) -> Option<String> {
        let Some(evidence) = exploration_evidence(tool_call, result) else {
            self.reset_result_streaks();
            return None;
        };

        if evidence.zero_matches {
            self.consecutive_zero_evidence_searches += 1;
        } else {
            self.consecutive_zero_evidence_searches = 0;
        }
        if evidence.truncated {
            self.consecutive_truncated_exploration += 1;
        } else {
            self.consecutive_truncated_exploration = 0;
        }

        if self.consecutive_zero_evidence_searches >= ZERO_EVIDENCE_SEARCH_THRESHOLD {
            self.warning_count += 1;
            self.consecutive_zero_evidence_searches = 0;
            return Some(
                "progress_guard: three consecutive searches returned zero matches. Stop varying broad terms: verify the path/scope and search contract, then use one targeted alternative or state that no evidence was found."
                    .to_string(),
            );
        }
        if self.consecutive_truncated_exploration >= TRUNCATED_EXPLORATION_THRESHOLD {
            self.warning_count += 1;
            self.consecutive_truncated_exploration = 0;
            return Some(
                "progress_guard: repeated exploration results were truncated. Narrow the query or path before requesting more output, then inspect the owning module and a small number of call sites."
                    .to_string(),
            );
        }
        None
    }

    fn reset_result_streaks(&mut self) {
        self.consecutive_zero_evidence_searches = 0;
        self.consecutive_truncated_exploration = 0;
    }

    fn reuse_unchanged_observation(
        &mut self,
        tool_call: &ToolCall,
        result: &mut ToolResult,
    ) -> Option<String> {
        if result.error.is_some() {
            return None;
        }
        let signature = exploration_signature(tool_call)?;
        let value = result.result.as_ref()?;
        let encoded = serde_json::to_vec(value).ok()?;
        let result_hash: [u8; 32] = Sha256::digest(&encoded).into();

        let unchanged = self.observation_hashes.get(&signature) == Some(&result_hash);
        self.remember_observation(signature.clone(), result_hash);
        if !unchanged || encoded.len() < MIN_REUSABLE_RESULT_BYTES {
            return None;
        }

        // This won against generic oversized-result summarization in the focused
        // discovery study: the first full result stays available, while an exact
        // repeat becomes a small freshness proof instead of another lossy summary.
        result.result = Some(json!({
            "unchanged_since_last_read": true,
            "tool": tool_call.name,
        }));
        self.warning_count += 1;
        Some(
            "progress_guard: this read/search result is unchanged since the same target was last inspected. Reuse the earlier full result; continue only if a different question or scope needs evidence."
                .to_string(),
        )
    }

    fn remember_observation(&mut self, signature: String, result_hash: [u8; 32]) {
        if self.observation_hashes.contains_key(&signature) {
            self.recent_observations
                .retain(|candidate| candidate != &signature);
        }
        self.observation_hashes
            .insert(signature.clone(), result_hash);
        self.recent_observations.push_back(signature);
        if self.recent_observations.len() > SEMANTIC_HISTORY_LIMIT
            && let Some(expired) = self.recent_observations.pop_front()
        {
            self.observation_hashes.remove(&expired);
        }
    }

    fn exploration_warning(&mut self) -> Option<String> {
        if self.exploration_since_progress == EXPLORATION_WITHOUT_PROGRESS_THRESHOLD {
            self.warning_count += 1;
            return Some(format!(
                "progress_guard: {EXPLORATION_WITHOUT_PROGRESS_THRESHOLD} investigation tools have run without an edit or validation. Narrow the hypothesis now: identify the exact missing evidence, make the smallest relevant change, or run one decisive verification command."
            ));
        }
        if self.exploration_since_progress >= CHECKPOINT_WITHOUT_PROGRESS_THRESHOLD {
            self.warning_count += 1;
            return Some(format!(
                "progress_guard: checkpoint required after {count} investigation tools without an edit or validation. Stop reading broadly and produce a checkpoint before more exploration: facts learned, current hypothesis, and the next decisive action (edit, validation, or no-change diagnosis).",
                count = self.exploration_since_progress
            ));
        }
        None
    }

    fn repetition_warning(&mut self, tool_call: &ToolCall) -> Option<String> {
        let Some(signature) = exploration_signature(tool_call) else {
            self.repeated_exploration_count = 0;
            self.last_exploration_signature = None;
            return None;
        };
        if self.last_exploration_signature.as_deref() == Some(signature.as_str()) {
            self.repeated_exploration_count += 1;
        } else {
            self.repeated_exploration_count = 1;
            self.last_exploration_signature = Some(signature);
        }
        if self.repeated_exploration_count >= REPEATED_EXPLORATION_THRESHOLD {
            self.warning_count += 1;
            self.repeated_exploration_count = 0;
            return Some(
                "progress_guard: repeated the same investigation target without an intervening edit or validation. Use the evidence already gathered, state the hypothesis, or switch to a decisive test/change."
                    .to_string(),
            );
        }
        None
    }
}

struct ProgressGuardHook {
    state: Arc<Mutex<ProgressGuardState>>,
}

#[async_trait]
impl PostToolExecHook for ProgressGuardHook {
    fn priority(&self) -> PostToolExecHookPriority {
        PostToolExecHookPriority::Normal
    }

    async fn after_exec(
        &self,
        tool_call: &ToolCall,
        _tool_def: &ToolDefinition,
        result: &mut ToolResult,
        context: &ToolContext,
    ) {
        let warning = {
            let mut state = self.state.lock().expect("progress guard state poisoned");
            let progress = state
                .sessions
                .entry(context.session_id.to_string())
                .or_default();
            progress.observe(tool_call, result)
        };

        if let Some(warning) = warning {
            inject_warning(result, warning);
        }
    }
}

#[derive(Clone, Copy)]
struct ExplorationEvidence {
    zero_matches: bool,
    truncated: bool,
}

fn exploration_evidence(tool_call: &ToolCall, result: &ToolResult) -> Option<ExplorationEvidence> {
    if result.error.is_some() {
        return None;
    }
    if !matches!(
        tool_call.name.as_str(),
        "grep_files" | "repo_map" | "ast_grep"
    ) {
        return None;
    }
    let value = result.result.as_ref()?;
    let count = value
        .get("count")
        .or_else(|| value.get("match_count"))
        .and_then(Value::as_u64)?;
    Some(ExplorationEvidence {
        zero_matches: count == 0,
        truncated: value
            .get("truncated")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

#[derive(Debug, PartialEq, Eq)]
enum ToolClass {
    Exploration,
    Mutation,
    Validation(String),
    Status(String),
    /// Checking on an external event (CI run, PR checks/reviews) or plain
    /// sleeping — the poll-loop shape that should instead be one detached
    /// `spawn_background` watch.
    Waiting,
    Other,
}

fn classify_tool_call(tool_call: &ToolCall) -> ToolClass {
    match tool_call.name.as_str() {
        "read_file" | "grep_files" | "repo_map" | "search_sessions" | "ast_grep"
        | "list_directory" | "stat_file" => ToolClass::Exploration,
        "write_file" | "edit_file" | "delete_file" | "ast_edit" => ToolClass::Mutation,
        "get_task" | "list_tasks" => ToolClass::Waiting,
        "bash" => classify_bash_command(
            tool_call
                .arguments
                .get("commands")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        ),
        _ => ToolClass::Other,
    }
}

fn classify_bash_command(command: &str) -> ToolClass {
    let normalized = normalize_command(command);
    if normalized.is_empty() {
        return ToolClass::Other;
    }
    // A leading `sleep N && …` is a poll delay: classify the probed command
    // (`sleep 5 && cargo test` is still validation). A bare `sleep` is pure
    // waiting.
    if let Some(tail) = poll_delay_tail(&normalized) {
        if tail.is_empty() {
            return ToolClass::Waiting;
        }
        return classify_bash_command(tail);
    }
    if is_waiting_command(&normalized) {
        return ToolClass::Waiting;
    }
    if is_status_command(&normalized) {
        return ToolClass::Status(normalized);
    }
    if is_validation_command(&normalized) {
        return ToolClass::Validation(normalized);
    }
    if is_mutating_command(&normalized) {
        return ToolClass::Mutation;
    }
    if is_exploration_command(&normalized) {
        return ToolClass::Exploration;
    }
    ToolClass::Other
}

struct MutationHashTransition {
    path: String,
    previous_hash: Option<String>,
    content_hash: String,
}

fn mutation_hash_transition(
    tool_call: &ToolCall,
    result: &ToolResult,
) -> Option<MutationHashTransition> {
    if !matches!(tool_call.name.as_str(), "edit_file" | "write_file") {
        return None;
    }
    let value = result.result.as_ref()?;
    Some(MutationHashTransition {
        path: value.get("path")?.as_str()?.to_string(),
        previous_hash: value
            .get("previous_content_hash")
            .and_then(Value::as_str)
            .map(str::to_string),
        content_hash: value.get("content_hash")?.as_str()?.to_string(),
    })
}

fn mutation_was_applied(tool_call: &ToolCall, result: &ToolResult) -> bool {
    if tool_call.name == "ast_edit" {
        return result
            .result
            .as_ref()
            .and_then(|value| value.get("applied"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
    }
    true
}

fn remember_bounded<T: Clone + Eq + Hash>(
    seen: &mut HashSet<T>,
    recent: &mut VecDeque<T>,
    value: T,
) {
    seen.insert(value.clone());
    recent.push_back(value);
    if recent.len() > SEMANTIC_HISTORY_LIMIT
        && let Some(expired) = recent.pop_front()
    {
        seen.remove(&expired);
    }
}

/// For a command starting with `sleep`, everything after the first `&&`/`;`
/// (empty when the sleep is the whole command). `None` when the command does
/// not start with a sleep.
fn poll_delay_tail(command: &str) -> Option<&str> {
    let rest = command.strip_prefix("sleep")?;
    if !rest.is_empty() && !rest.starts_with(' ') {
        return None;
    }
    let tail = rest
        .find("&&")
        .map(|at| &rest[at + 2..])
        .or_else(|| rest.find(';').map(|at| &rest[at + 1..]))
        .unwrap_or("");
    Some(tail.trim())
}

fn normalize_command(command: &str) -> String {
    command.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn exploration_signature(tool_call: &ToolCall) -> Option<String> {
    match tool_call.name.as_str() {
        "read_file" => {
            let path = tool_call.arguments.get("path").and_then(Value::as_str)?;
            let offset = tool_call
                .arguments
                .get("offset")
                .and_then(Value::as_i64)
                .unwrap_or(0);
            Some(format!("read_file:{path}:{offset}"))
        }
        "grep_files" => {
            let pattern = tool_call
                .arguments
                .get("pattern")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let path_pattern = tool_call
                .arguments
                .get("path_pattern")
                .and_then(Value::as_str)
                .unwrap_or_default();
            Some(format!("grep_files:{path_pattern}:{pattern}"))
        }
        "repo_map" | "search_sessions" | "ast_grep" | "list_directory" | "stat_file" => {
            Some(format!(
                "{}:{}",
                tool_call.name,
                normalize_value(&tool_call.arguments)
            ))
        }
        "bash" => {
            let command = tool_call
                .arguments
                .get("commands")
                .and_then(Value::as_str)
                .map(normalize_command)
                .unwrap_or_default();
            is_exploration_command(&command).then(|| format!("bash:{command}"))
        }
        _ => None,
    }
}

fn normalize_value(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| value.to_string())
}

fn is_waiting_command(command: &str) -> bool {
    let prefixes = [
        "gh pr checks",
        "gh pr status",
        "gh pr view",
        "gh run list",
        "gh run view",
        "gh run watch",
        "gh workflow view",
    ];
    prefixes.iter().any(|prefix| command.starts_with(prefix))
}

fn is_status_command(command: &str) -> bool {
    matches!(
        command,
        "git status" | "git status --short" | "git status --short --branch" | "git diff"
    ) || command.starts_with("git diff ")
        || command.starts_with("git status ")
}

fn is_validation_command(command: &str) -> bool {
    let prefixes = [
        "cargo test",
        "cargo check",
        "cargo build",
        "cargo clippy",
        "cargo fmt --check",
        "npm test",
        "npm run test",
        "pnpm test",
        "pnpm run test",
        "yarn test",
        "pytest",
        "uv run",
        "go test",
        "python -m unittest",
    ];
    prefixes.iter().any(|prefix| command.starts_with(prefix))
}

fn is_mutating_command(command: &str) -> bool {
    let tokens = [
        "apply_patch",
        "cargo fmt",
        "cargo update",
        "cargo generate-lockfile",
        "cargo add",
        "cargo remove",
        "cargo fix",
        "npm run format",
        "pnpm run format",
        "git apply",
        "git commit",
        "git add",
        "mv ",
        "cp ",
        "rm ",
        "mkdir ",
    ];
    tokens.iter().any(|token| command.contains(token))
}

fn is_exploration_command(command: &str) -> bool {
    let prefixes = [
        "rg ",
        "grep ",
        "find ",
        "sed ",
        "cat ",
        "ls",
        "git show",
        "git log",
        "git blame",
        "git grep",
        "git ls-files",
    ];
    prefixes.iter().any(|prefix| command.starts_with(prefix))
}

fn inject_warning(result: &mut ToolResult, warning: String) {
    let mut object = match result.result.take() {
        Some(Value::Object(object)) => object,
        Some(value) => {
            let mut object = Map::new();
            object.insert("result".to_string(), value);
            object
        }
        None => Map::new(),
    };
    object.insert("progress_guard_warning".to_string(), json!(warning));
    result.result = Some(Value::Object(object));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool_types::{BuiltinTool, DeferrablePolicy, ToolHints, ToolPolicy, ToolResult};
    use crate::typed_id::SessionId;

    fn call(name: &str, arguments: Value) -> ToolCall {
        ToolCall {
            id: format!("call-{name}"),
            name: name.to_string(),
            arguments,
        }
    }

    fn tool_def(name: &str) -> ToolDefinition {
        ToolDefinition::Builtin(BuiltinTool {
            name: name.to_string(),
            display_name: None,
            description: "test".to_string(),
            parameters: json!({ "type": "object" }),
            policy: ToolPolicy::Auto,
            category: None,
            deferrable: DeferrablePolicy::Never,
            hints: ToolHints::default(),
            full_parameters: None,
        })
    }

    fn result() -> ToolResult {
        ToolResult {
            tool_call_id: "call".to_string(),
            result: Some(json!({ "ok": true })),
            images: None,
            error: None,
            connection_required: None,
            raw_output: None,
        }
    }

    fn result_value(value: Value) -> ToolResult {
        ToolResult {
            result: Some(value),
            ..result()
        }
    }

    #[tokio::test]
    async fn unchanged_large_read_returns_compact_marker_and_mutation_invalidates_it() {
        let state = Arc::new(Mutex::new(ProgressGuardState::default()));
        let hook = ProgressGuardHook { state };
        let context = ToolContext::new(SessionId::new());
        let read = call("read_file", json!({ "path": "/src/lib.rs" }));
        let payload = json!({ "content": "discovery evidence\n".repeat(400) });

        let mut first = result_value(payload.clone());
        hook.after_exec(&read, &tool_def("read_file"), &mut first, &context)
            .await;
        let first_bytes = serde_json::to_vec(first.result.as_ref().unwrap())
            .unwrap()
            .len();

        let mut repeated = result_value(payload.clone());
        hook.after_exec(&read, &tool_def("read_file"), &mut repeated, &context)
            .await;
        let repeated_value = repeated.result.as_ref().unwrap();
        let repeated_bytes = serde_json::to_vec(repeated_value).unwrap().len();
        assert_eq!(repeated_value["unchanged_since_last_read"], true);
        assert!(
            repeated_value["progress_guard_warning"]
                .as_str()
                .is_some_and(|warning| warning.contains("earlier full result"))
        );
        assert!(
            repeated_bytes * 10 < first_bytes,
            "compact unchanged marker should materially cut context bytes: {repeated_bytes} vs {first_bytes}"
        );

        let mut write = result();
        hook.after_exec(
            &call(
                "write_file",
                json!({ "path": "/src/lib.rs", "content": "changed" }),
            ),
            &tool_def("write_file"),
            &mut write,
            &context,
        )
        .await;
        let mut after_mutation = result_value(payload);
        hook.after_exec(&read, &tool_def("read_file"), &mut after_mutation, &context)
            .await;
        assert!(after_mutation.result.unwrap().get("content").is_some());
    }

    #[tokio::test]
    async fn reuse_requires_the_same_target_and_same_result() {
        let state = Arc::new(Mutex::new(ProgressGuardState::default()));
        let hook = ProgressGuardHook { state };
        let context = ToolContext::new(SessionId::new());
        let large = "x".repeat(2_000);

        let cases = [
            ("/a.rs", large.clone()),
            ("/b.rs", large.clone()),
            ("/a.rs", format!("{large} changed")),
        ];
        for (path, content) in cases {
            let mut observed = result_value(json!({ "content": content }));
            hook.after_exec(
                &call("read_file", json!({ "path": path })),
                &tool_def("read_file"),
                &mut observed,
                &context,
            )
            .await;
            assert!(
                observed
                    .result
                    .as_ref()
                    .and_then(|value| value.get("unchanged_since_last_read"))
                    .is_none(),
                "different targets or changed bytes are not reusable"
            );
        }
    }

    #[test]
    fn classify_bash_command_distinguishes_status_and_validation() {
        assert_eq!(
            classify_bash_command("git status --short --branch"),
            ToolClass::Status("git status --short --branch".to_string())
        );
        assert_eq!(
            classify_bash_command("cargo test --all-features"),
            ToolClass::Validation("cargo test --all-features".to_string())
        );
        assert_eq!(
            classify_bash_command("cargo check --all-features"),
            ToolClass::Validation("cargo check --all-features".to_string())
        );
        assert_eq!(
            classify_bash_command("cargo update -p everruns-core --precise 0.17.7"),
            ToolClass::Mutation
        );
        assert_eq!(
            classify_bash_command("rg progress_guard"),
            ToolClass::Exploration
        );
    }

    #[test]
    fn classify_bash_command_detects_external_event_waits() {
        assert_eq!(classify_bash_command("gh pr checks 42"), ToolClass::Waiting);
        assert_eq!(
            classify_bash_command("gh run list --branch main --limit 5"),
            ToolClass::Waiting
        );
        assert_eq!(classify_bash_command("sleep 120"), ToolClass::Waiting);
        assert_eq!(
            classify_bash_command("sleep 30 && gh pr checks 42"),
            ToolClass::Waiting
        );
        // A sleep in front of real work classifies as the work itself.
        assert_eq!(
            classify_bash_command("sleep 5 && cargo test --all-features"),
            ToolClass::Validation("cargo test --all-features".to_string())
        );
        // `sleepwalk` must not parse as a sleep prefix.
        assert_eq!(classify_bash_command("sleepwalk"), ToolClass::Other);
    }

    #[tokio::test]
    async fn hook_warns_after_long_exploration_without_progress() {
        let state = Arc::new(Mutex::new(ProgressGuardState::default()));
        let hook = ProgressGuardHook { state };
        let context = ToolContext::new(SessionId::new());
        let mut last = result();

        for _ in 0..EXPLORATION_WITHOUT_PROGRESS_THRESHOLD {
            last = result();
            hook.after_exec(
                &call("read_file", json!({ "path": "/src/lib.rs" })),
                &tool_def("read_file"),
                &mut last,
                &context,
            )
            .await;
        }

        assert!(
            last.result
                .as_ref()
                .and_then(|value| value.get("progress_guard_warning"))
                .and_then(Value::as_str)
                .is_some_and(|warning| warning.contains("investigation tools"))
        );
    }

    #[tokio::test]
    async fn hook_escalates_to_checkpoint_after_more_exploration() {
        let state = Arc::new(Mutex::new(ProgressGuardState::default()));
        let hook = ProgressGuardHook { state };
        let context = ToolContext::new(SessionId::new());
        let mut last = result();

        for i in 0..CHECKPOINT_WITHOUT_PROGRESS_THRESHOLD {
            last = result();
            hook.after_exec(
                &call("read_file", json!({ "path": format!("/src/{i}.rs") })),
                &tool_def("read_file"),
                &mut last,
                &context,
            )
            .await;
        }

        assert!(
            last.result
                .as_ref()
                .and_then(|value| value.get("progress_guard_warning"))
                .and_then(Value::as_str)
                .is_some_and(|warning| warning.contains("checkpoint required"))
        );
    }

    #[tokio::test]
    async fn hook_keeps_warning_after_checkpoint_until_progress() {
        let state = Arc::new(Mutex::new(ProgressGuardState::default()));
        let hook = ProgressGuardHook { state };
        let context = ToolContext::new(SessionId::new());
        let mut last = result();

        for i in 0..=CHECKPOINT_WITHOUT_PROGRESS_THRESHOLD {
            last = result();
            hook.after_exec(
                &call("grep_files", json!({ "pattern": format!("needle{i}") })),
                &tool_def("grep_files"),
                &mut last,
                &context,
            )
            .await;
        }

        assert!(
            last.result
                .as_ref()
                .and_then(|value| value.get("progress_guard_warning"))
                .and_then(Value::as_str)
                .is_some_and(|warning| warning.contains("checkpoint required"))
        );
    }

    #[tokio::test]
    async fn mutation_resets_exploration_warning_counter() {
        let state = Arc::new(Mutex::new(ProgressGuardState::default()));
        let hook = ProgressGuardHook { state };
        let context = ToolContext::new(SessionId::new());
        let mut last = result();

        for _ in 0..(EXPLORATION_WITHOUT_PROGRESS_THRESHOLD - 1) {
            hook.after_exec(
                &call("read_file", json!({ "path": "/src/lib.rs" })),
                &tool_def("read_file"),
                &mut result(),
                &context,
            )
            .await;
        }
        hook.after_exec(
            &call("edit_file", json!({ "path": "/src/lib.rs" })),
            &tool_def("edit_file"),
            &mut result(),
            &context,
        )
        .await;
        for _ in 0..(EXPLORATION_WITHOUT_PROGRESS_THRESHOLD - 1) {
            last = result();
            hook.after_exec(
                &call("read_file", json!({ "path": "/src/lib.rs" })),
                &tool_def("read_file"),
                &mut last,
                &context,
            )
            .await;
        }

        assert!(
            last.result
                .as_ref()
                .and_then(|value| value.get("progress_guard_warning"))
                .is_none()
        );
    }

    #[tokio::test]
    async fn validation_resets_checkpoint_warning_counter() {
        let state = Arc::new(Mutex::new(ProgressGuardState::default()));
        let hook = ProgressGuardHook { state };
        let context = ToolContext::new(SessionId::new());
        let mut last = result();

        for i in 0..CHECKPOINT_WITHOUT_PROGRESS_THRESHOLD {
            hook.after_exec(
                &call("read_file", json!({ "path": format!("/src/{i}.rs") })),
                &tool_def("read_file"),
                &mut result(),
                &context,
            )
            .await;
        }
        hook.after_exec(
            &call("bash", json!({ "commands": "cargo test --all-features" })),
            &tool_def("bash"),
            &mut result(),
            &context,
        )
        .await;
        for i in 0..(EXPLORATION_WITHOUT_PROGRESS_THRESHOLD - 1) {
            last = result();
            hook.after_exec(
                &call("read_file", json!({ "path": format!("/src/after/{i}.rs") })),
                &tool_def("read_file"),
                &mut last,
                &context,
            )
            .await;
        }

        assert!(
            last.result
                .as_ref()
                .and_then(|value| value.get("progress_guard_warning"))
                .is_none()
        );
    }

    #[tokio::test]
    async fn workspace_state_revisit_warns_on_a_mutation_cycle() {
        let state = Arc::new(Mutex::new(ProgressGuardState::default()));
        let hook = ProgressGuardHook { state };
        let context = ToolContext::new(SessionId::new());
        let transitions = [("A", "B"), ("B", "C"), ("C", "A")];

        for (index, (previous, current)) in transitions.into_iter().enumerate() {
            let mut out = result_value(json!({
                "path": "/workspace/Cargo.toml",
                "previous_content_hash": previous,
                "content_hash": current,
            }));
            hook.after_exec(
                &call("edit_file", json!({ "path": "/workspace/Cargo.toml" })),
                &tool_def("edit_file"),
                &mut out,
                &context,
            )
            .await;

            let warning = out
                .result
                .as_ref()
                .and_then(|value| value.get("progress_guard_warning"))
                .and_then(Value::as_str);
            if index < 2 {
                assert!(warning.is_none(), "new states are progress");
            } else {
                assert!(
                    warning.is_some_and(|text| text.contains("workspace state")),
                    "returning to A should expose the mutation cycle"
                );
            }
        }
    }

    #[tokio::test]
    async fn lockfile_updates_do_not_hide_a_manifest_state_cycle() {
        let state = Arc::new(Mutex::new(ProgressGuardState::default()));
        let hook = ProgressGuardHook { state };
        let context = ToolContext::new(SessionId::new());
        let transitions = [("A", "B"), ("B", "C"), ("C", "A")];

        for (index, (previous, current)) in transitions.into_iter().enumerate() {
            let mut edit = result_value(json!({
                "path": "/workspace/Cargo.toml",
                "previous_content_hash": previous,
                "content_hash": current,
            }));
            hook.after_exec(
                &call("edit_file", json!({ "path": "/workspace/Cargo.toml" })),
                &tool_def("edit_file"),
                &mut edit,
                &context,
            )
            .await;

            let warning = edit
                .result
                .as_ref()
                .and_then(|value| value.get("progress_guard_warning"))
                .and_then(Value::as_str);
            if index < 2 {
                assert!(warning.is_none());
            } else {
                assert!(warning.is_some_and(|text| text.contains("workspace state")));
            }

            let mut update = result();
            hook.after_exec(
                &call(
                    "bash",
                    json!({ "commands": "cargo update -p everruns-core --precise 0.17.7" }),
                ),
                &tool_def("bash"),
                &mut update,
                &context,
            )
            .await;
        }
    }

    #[tokio::test]
    async fn lockfile_update_makes_repeated_validation_fresh() {
        let state = Arc::new(Mutex::new(ProgressGuardState::default()));
        let hook = ProgressGuardHook { state };
        let context = ToolContext::new(SessionId::new());
        let validation = call("bash", json!({ "commands": "cargo check" }));

        hook.after_exec(&validation, &tool_def("bash"), &mut result(), &context)
            .await;
        hook.after_exec(
            &call(
                "bash",
                json!({ "commands": "cargo update -p everruns-core --precise 0.17.7" }),
            ),
            &tool_def("bash"),
            &mut result(),
            &context,
        )
        .await;

        let mut after_update = result();
        hook.after_exec(&validation, &tool_def("bash"), &mut after_update, &context)
            .await;
        assert!(
            after_update
                .result
                .as_ref()
                .and_then(|value| value.get("progress_guard_warning"))
                .is_none(),
            "validation after a lockfile mutation has new workspace evidence"
        );

        let mut unchanged_again = result();
        hook.after_exec(
            &validation,
            &tool_def("bash"),
            &mut unchanged_again,
            &context,
        )
        .await;
        assert!(
            unchanged_again
                .result
                .as_ref()
                .and_then(|value| value.get("progress_guard_warning"))
                .is_some(),
            "a second validation without another mutation is redundant"
        );
    }

    #[tokio::test]
    async fn repeated_validation_on_unchanged_state_warns() {
        let state = Arc::new(Mutex::new(ProgressGuardState::default()));
        let hook = ProgressGuardHook { state };
        let context = ToolContext::new(SessionId::new());
        let validation = call("bash", json!({ "commands": "cargo test" }));

        let mut first = result();
        hook.after_exec(&validation, &tool_def("bash"), &mut first, &context)
            .await;
        assert!(
            first
                .result
                .as_ref()
                .and_then(|value| value.get("progress_guard_warning"))
                .is_none()
        );

        let mut repeated = result();
        hook.after_exec(&validation, &tool_def("bash"), &mut repeated, &context)
            .await;
        assert!(
            repeated
                .result
                .as_ref()
                .and_then(|value| value.get("progress_guard_warning"))
                .and_then(Value::as_str)
                .is_some_and(|text| text.contains("unchanged workspace state"))
        );

        let mut edit = result_value(json!({
            "path": "/workspace/src/lib.rs",
            "previous_content_hash": "A",
            "content_hash": "B",
        }));
        hook.after_exec(
            &call("edit_file", json!({ "path": "/workspace/src/lib.rs" })),
            &tool_def("edit_file"),
            &mut edit,
            &context,
        )
        .await;

        let mut after_progress = result();
        hook.after_exec(
            &validation,
            &tool_def("bash"),
            &mut after_progress,
            &context,
        )
        .await;
        assert!(
            after_progress
                .result
                .as_ref()
                .and_then(|value| value.get("progress_guard_warning"))
                .is_none(),
            "the same validation is useful after the workspace changes"
        );
    }

    #[tokio::test]
    async fn semantic_progress_has_no_fixed_session_iteration_limit() {
        let state = Arc::new(Mutex::new(ProgressGuardState::default()));
        let hook = ProgressGuardHook { state };
        let context = ToolContext::new(SessionId::new());

        for index in 0..256 {
            let mut out = result_value(json!({
                "path": "/workspace/src/lib.rs",
                "previous_content_hash": format!("state-{index}"),
                "content_hash": format!("state-{}", index + 1),
            }));
            hook.after_exec(
                &call("edit_file", json!({ "path": "/workspace/src/lib.rs" })),
                &tool_def("edit_file"),
                &mut out,
                &context,
            )
            .await;
            assert!(
                out.result
                    .as_ref()
                    .and_then(|value| value.get("progress_guard_warning"))
                    .is_none(),
                "each new state remains progress after iteration {index}"
            );
        }
    }

    #[tokio::test]
    async fn repeated_exploration_target_warns() {
        let state = Arc::new(Mutex::new(ProgressGuardState::default()));
        let hook = ProgressGuardHook { state };
        let context = ToolContext::new(SessionId::new());
        let mut last = result();

        for _ in 0..REPEATED_EXPLORATION_THRESHOLD {
            last = result();
            hook.after_exec(
                &call("read_file", json!({ "path": "/src/lib.rs", "offset": 10 })),
                &tool_def("read_file"),
                &mut last,
                &context,
            )
            .await;
        }

        assert!(
            last.result
                .as_ref()
                .and_then(|value| value.get("progress_guard_warning"))
                .and_then(Value::as_str)
                .is_some_and(|warning| warning.contains("same investigation target"))
        );
    }

    #[tokio::test]
    async fn three_zero_evidence_searches_warn_even_when_queries_differ() {
        let state = Arc::new(Mutex::new(ProgressGuardState::default()));
        let hook = ProgressGuardHook { state };
        let context = ToolContext::new(SessionId::new());
        let mut last = result();

        for pattern in ["history", "ground", "session"] {
            last = result_value(json!({ "ok": true, "count": 0, "matches": [] }));
            hook.after_exec(
                &call("grep_files", json!({ "pattern": pattern })),
                &tool_def("grep_files"),
                &mut last,
                &context,
            )
            .await;
        }

        assert!(
            last.result
                .as_ref()
                .and_then(|value| value.get("progress_guard_warning"))
                .and_then(Value::as_str)
                .is_some_and(|warning| warning.contains("zero matches"))
        );
    }

    #[tokio::test]
    async fn positive_search_evidence_resets_zero_result_streak() {
        let state = Arc::new(Mutex::new(ProgressGuardState::default()));
        let hook = ProgressGuardHook { state };
        let context = ToolContext::new(SessionId::new());

        for count in [0, 0, 1, 0, 0] {
            let mut out = result_value(json!({ "ok": true, "count": count }));
            hook.after_exec(
                &call("repo_map", json!({ "query": format!("query-{count}") })),
                &tool_def("repo_map"),
                &mut out,
                &context,
            )
            .await;
            assert!(
                out.result
                    .as_ref()
                    .and_then(|value| value.get("progress_guard_warning"))
                    .is_none(),
                "a positive result should break the zero-evidence streak"
            );
        }
    }

    #[tokio::test]
    async fn repeated_truncated_exploration_warns_to_narrow_scope() {
        let state = Arc::new(Mutex::new(ProgressGuardState::default()));
        let hook = ProgressGuardHook { state };
        let context = ToolContext::new(SessionId::new());
        let mut last = result();

        for query in ["runtime", "capability"] {
            last = result_value(json!({ "ok": true, "count": 50, "truncated": true }));
            hook.after_exec(
                &call("repo_map", json!({ "query": query })),
                &tool_def("repo_map"),
                &mut last,
                &context,
            )
            .await;
        }

        assert!(
            last.result
                .as_ref()
                .and_then(|value| value.get("progress_guard_warning"))
                .and_then(Value::as_str)
                .is_some_and(|warning| warning.contains("truncated"))
        );
    }

    #[tokio::test]
    async fn original_session_pattern_gets_interrupted_before_runaway_reads() {
        let state = Arc::new(Mutex::new(ProgressGuardState::default()));
        let hook = ProgressGuardHook { state };
        let context = ToolContext::new(SessionId::new());
        let mut warnings = Vec::new();

        let observe = |tool_call: ToolCall| {
            let hook = &hook;
            let context = &context;
            async move {
                let mut out = result();
                hook.after_exec(&tool_call, &tool_def(&tool_call.name), &mut out, context)
                    .await;
                out.result
                    .as_ref()
                    .and_then(|value| value.get("progress_guard_warning"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
            }
        };

        // Same shape as the failed investigation: broad searches, then many
        // repeated reads of the callback-adjacent files, with no edit/test.
        let broad_searches = [
            "scheduled|schedule|signal_on_completion|callback|background",
            "spawn_background|signal_on_completion|scheduled_at",
            "cron|completion|task|background_run|Background|Task",
            "drain_finished_for_wake|wake_prompt|BackgroundRegistry",
            "SessionScheduleStore|schedule_store|create_schedule",
            "maybe_wake_for_background|TaskRegistryEvent",
        ];
        for pattern in broad_searches {
            if let Some(warning) = observe(call(
                "grep_files",
                json!({ "pattern": pattern, "path_pattern": "src/**/*.rs" }),
            ))
            .await
            {
                warnings.push(warning);
            }
        }
        for offset in [180, 388, 633, 940, 1370, 1540, 180, 388, 633] {
            if let Some(warning) = observe(call(
                "read_file",
                json!({ "path": "/repo/src/capabilities/background.rs", "offset": offset }),
            ))
            .await
            {
                warnings.push(warning);
            }
        }
        for offset in [1000, 1020, 1010, 1028, 1000, 1020, 1010, 1028, 1000] {
            if let Some(warning) = observe(call(
                "read_file",
                json!({ "path": "/repo/src/app/mod.rs", "offset": offset }),
            ))
            .await
            {
                warnings.push(warning);
            }
        }
        assert!(
            warnings
                .iter()
                .any(|warning| warning.contains("investigation tools")),
            "first threshold should warn before the session keeps circling: {warnings:?}"
        );

        for i in 0..24 {
            if let Some(warning) = observe(call(
                "read_file",
                json!({ "path": "/repo/src/runtime.rs", "offset": 2100 + i }),
            ))
            .await
            {
                warnings.push(warning);
            }
        }
        assert!(
            warnings
                .iter()
                .any(|warning| warning.contains("checkpoint required")),
            "checkpoint escalation should trigger by 48 read/search calls: {warnings:?}"
        );

        for _ in 0..REPEATED_EXPLORATION_THRESHOLD {
            if let Some(warning) = observe(call(
                "read_file",
                json!({ "path": "/repo/src/app/mod.rs", "offset": 1000 }),
            ))
            .await
            {
                warnings.push(warning);
            }
        }
        assert!(
            warnings
                .iter()
                .any(|warning| warning.contains("same investigation target")),
            "semantic repetition should catch rereading the same range: {warnings:?}"
        );
    }

    #[tokio::test]
    async fn repeated_ci_polling_warns_toward_spawn_background() {
        let state = Arc::new(Mutex::new(ProgressGuardState::default()));
        let hook = ProgressGuardHook { state };
        let context = ToolContext::new(SessionId::new());
        let mut last = result();

        // The classic poll loop: delay-then-check, over and over. Different
        // probe commands still count — polling rarely repeats verbatim.
        let polls = [
            "gh pr checks 42",
            "sleep 30 && gh pr checks 42",
            "gh run list --limit 1",
            "gh pr view 42",
        ];
        for command in polls {
            last = result();
            hook.after_exec(
                &call("bash", json!({ "commands": command })),
                &tool_def("bash"),
                &mut last,
                &context,
            )
            .await;
        }

        assert!(
            last.result
                .as_ref()
                .and_then(|value| value.get("progress_guard_warning"))
                .and_then(Value::as_str)
                .is_some_and(|warning| warning.contains("spawn_background"))
        );
    }

    #[tokio::test]
    async fn semantic_polling_cycle_warns_across_heterogeneous_tools() {
        let state = Arc::new(Mutex::new(ProgressGuardState::default()));
        let hook = ProgressGuardHook { state };
        let context = ToolContext::new(SessionId::new());
        let cycle = [
            call("bash", json!({ "commands": "gh pr checks 42" })),
            call("read_file", json!({ "path": "/tmp/synthetic-ci-note" })),
            call("get_task", json!({ "task_id": "task_ci" })),
            call("bash", json!({ "commands": "gh run view 123" })),
        ];
        let mut warnings = Vec::new();

        for tool_call in cycle.into_iter().cycle().take(8) {
            let mut output = result();
            hook.after_exec(
                &tool_call,
                &tool_def(&tool_call.name),
                &mut output,
                &context,
            )
            .await;
            if let Some(warning) = output
                .result
                .as_ref()
                .and_then(|value| value.get("progress_guard_warning"))
                .and_then(Value::as_str)
            {
                warnings.push(warning.to_string());
            }
        }

        assert!(
            warnings
                .iter()
                .any(|warning| warning.contains("spawn_background")),
            "a semantic polling cycle must be caught even when unrelated reads and task probes separate external checks: {warnings:?}"
        );
    }

    #[tokio::test]
    async fn one_off_task_and_ci_status_checks_do_not_warn() {
        let state = Arc::new(Mutex::new(ProgressGuardState::default()));
        let hook = ProgressGuardHook { state };
        let context = ToolContext::new(SessionId::new());

        for tool_call in [
            call("list_tasks", json!({})),
            call("get_task", json!({ "task_id": "task_once" })),
            call("bash", json!({ "commands": "gh pr checks 42" })),
        ] {
            let mut output = result();
            hook.after_exec(
                &tool_call,
                &tool_def(&tool_call.name),
                &mut output,
                &context,
            )
            .await;
            assert!(
                output
                    .result
                    .as_ref()
                    .and_then(|value| value.get("progress_guard_warning"))
                    .is_none(),
                "a one-off status check is legitimate"
            );
        }
    }

    #[tokio::test]
    async fn interleaved_work_resets_waiting_window() {
        let state = Arc::new(Mutex::new(ProgressGuardState::default()));
        let hook = ProgressGuardHook { state };
        let context = ToolContext::new(SessionId::new());

        // Check CI, fix something, check again — legitimate, because real
        // progress clears the semantic window.
        for _ in 0..(WAITING_WINDOW_THRESHOLD * 2) {
            let mut check = result();
            hook.after_exec(
                &call("bash", json!({ "commands": "gh pr checks 42" })),
                &tool_def("bash"),
                &mut check,
                &context,
            )
            .await;
            assert!(
                check
                    .result
                    .as_ref()
                    .and_then(|value| value.get("progress_guard_warning"))
                    .is_none()
            );
            hook.after_exec(
                &call("edit_file", json!({ "path": "/src/lib.rs" })),
                &tool_def("edit_file"),
                &mut result(),
                &context,
            )
            .await;
        }
    }

    #[tokio::test]
    async fn repeated_git_status_warns() {
        let state = Arc::new(Mutex::new(ProgressGuardState::default()));
        let hook = ProgressGuardHook { state };
        let context = ToolContext::new(SessionId::new());
        let mut last = result();

        for _ in 0..REPEATED_STATUS_THRESHOLD {
            last = result();
            hook.after_exec(
                &call("bash", json!({ "commands": "git status --short" })),
                &tool_def("bash"),
                &mut last,
                &context,
            )
            .await;
        }

        assert!(
            last.result
                .as_ref()
                .and_then(|value| value.get("progress_guard_warning"))
                .and_then(Value::as_str)
                .is_some_and(|warning| warning.contains("repeated git status"))
        );
    }
}
