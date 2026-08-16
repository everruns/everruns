---
type: Specification
title: "User Hooks Specification"
description: "User-authored lifecycle hooks for agent execution."
tags:
  - everruns
  - runtime-resources
---
# User Hooks Specification

## Abstract

User hooks let operators inject shell commands at well-defined points in the
agent execution lifecycle. The user authors a small JSON config; the runtime
runs the configured commands at the right moment, delivers a structured JSON
payload via environment variables plus a session-VFS payload file (the bash
executor has no process stdin, see "Wire contract" below), and reads a
structured decision from stdout. Hooks can mutate the inputs they observe or
block execution outright.

User hooks are exposed through the `user_hooks` capability and are also
contributable by any other capability (built-in, MCP, or declarative). Multiple
contributors compose into a single ordered chain per event.

## Goals

- A single, small, finite event taxonomy that covers the high-value
  pre/post points without exploding into per-internal-detail callbacks.
- One executor contract today (bash, sandboxed via `bashkit_shell`), designed so
  webhook/wasm/blueprint backends slot in later without API churn.
- Hooks are *data*: a `UserHookSpec` is a JSON-serializable spec the runtime
  translates into adapter `Arc<dyn …>` values. No capability hands the runtime
  a raw executor.
- Composable across contributors. Capability-declaration order is the chain
  order. The first `Block` decision wins.
- Honest trust model: enabling a capability that ships hooks rides the same
  trust gate as enabling any other behavior-bearing capability.

## Non-goals (v1)

- Hooks that run on `pre_model_call` / `post_model_call`. Overlaps with
  `OutputGuardrail` plus has cost concerns. Revisit once the streaming
  surface stabilizes.
- Hooks that mutate the tool definition list (`tool_definition_hooks`). Too
  easy to break agent reliability from a shell script.
- Hooks targeted at subagents, compaction, or PR/issue notifications.
- A general-purpose webhook backend, wasm sandbox, or LLM-as-hook backend.
  The trait accommodates these but v1 ships only the bash backend.

## Event taxonomy

Six events. Each maps to a Rust trait. This PR ships the *contract,
types, executor, capability, and config validation* for all six. The
runtime firing point (the "wire-in") for each event is a separate, narrow
change tracked as a follow-up so reviewers can audit it against the
relevant execution atom in isolation.

| Event | Trait | Fires at | Mutation surface | Can block? | Wire-in status |
|---|---|---|---|---|---|
| `session_start` | `SessionLifecycleHook` | session create | none | no | **wired** in `SessionService::create_inner` (server) |
| `user_prompt_submit` | `TurnLifecycleHook` | first reason iteration, before the LLM | user message text | yes | **wired** in `execute_reason_activity` (runtime) |
| `pre_tool_use` | `PreToolUseHook::before_exec` | each tool call, before execution | `ToolCall.arguments` (JSON patch) | yes | **wired** in `ActAtom::execute_single_tool` |
| `post_tool_use` | `PostToolExecHook::after_exec` (existing trait) | each tool call, after execution | `ToolResult` fields | no | **wired** via existing `PostToolExecHook` chain |
| `turn_end` | `TurnLifecycleHook` | turn finishes | none | no | **wired** in `RuntimeSessionLifecycle::fire_turn_end_hooks` |
| `session_end` | `SessionLifecycleHook` | session delete | none | no | **wired** in `SessionService::delete` (server) |

All six events now fire. `user_prompt_submit` fires on the first reason
iteration (the shared choke point for both the in-process loop and the
durable worker, immediately before the LLM is consulted) rather than at raw
HTTP submission: the server returns the stored message to the caller and runs
the turn asynchronously, so a `Block` aborts the turn (emitting a user-facing
message + `turn.failed` and idling the session) rather than rejecting the
original HTTP response.

This phasing traded a slightly longer rollout for cleaner reviewability:
each wire-in is a 50-100 LOC change against a single execution atom with
focused tests, instead of one mega-PR touching every atom.

Block semantics:

- `session_start` and `session_end` are advisory only. A hook may fail
  internally; that is logged but never aborts the session.
- `user_prompt_submit` can block: the user message is rejected, the turn
  is aborted, and the configured `user_message` (if any) is surfaced.
- `pre_tool_use` can block: the tool returns immediately with an error
  result containing the hook's reason. The chain stops at the first block.
- `post_tool_use` cannot block, by the time it runs the tool has already
  executed and the model can't unsee that. It can only mutate the result.

## Wire contract: bash executor

Bash is the v1 executor. The contract is shaped after Claude Code hooks with
one deliberate departure: payload delivery uses env vars + a session-VFS
payload file rather than process stdin, because the bashkit interpreter
does not expose process-level stdin to user scripts (bashkit interprets the
script in-process; there is no spawned shell to write to). Scripts that
prefer a `cat | jq` flow can `cat "$EVERRUNS_HOOK_PAYLOAD_PATH" | jq`.

### Input

Every invocation carries this envelope. The shape is identical across the
env-var (`EVERRUNS_HOOK_PAYLOAD_JSON`) and file
(`EVERRUNS_HOOK_PAYLOAD_PATH`) delivery channels:

```json
{
  "event": "pre_tool_use",
### Input — payload delivery

The hook script receives the payload via three sources, redundantly, so
authors can pick whichever fits their script style:

1. **`$EVERRUNS_HOOK_PAYLOAD_JSON`** — the full payload, JSON-encoded as
   a single string. The canonical source. Read with
   `jq -n 'env.EVERRUNS_HOOK_PAYLOAD_JSON | fromjson'` or
   `printf '%s' "$EVERRUNS_HOOK_PAYLOAD_JSON" | jq`.
2. **`$EVERRUNS_HOOK_PAYLOAD_PATH`** — path to the same payload written
   to the session VFS (default `/.hooks/payload-<call-id>.json`). The
   dispatcher cleans this up after the hook returns. Useful when the
   payload is large enough to be cumbersome in an env var.
3. **Convenience env vars** — `EVERRUNS_HOOK_EVENT`,
   `EVERRUNS_HOOK_ID`, `EVERRUNS_HOOK_SESSION_ID`,
   `EVERRUNS_HOOK_TURN_ID`, `EVERRUNS_HOOK_TOOL_NAME` (tool events
   only), `EVERRUNS_HOOK_TOOL_CALL_ID` (tool events only). Set unset
   when not applicable.

Why not stdin? The bashkit interpreter does not expose a process-level
stdin to user scripts (it interprets the script directly rather than
spawning a shell process). Env-var delivery is the closest equivalent
that preserves sandbox isolation. Scripts that want a `cat | jq`-style
flow can use `cat "$EVERRUNS_HOOK_PAYLOAD_PATH" | jq`.

Payload schema (the same object both env-var and file carry):

```json
{
  "event": "pre_tool_use",
  "hook_id": "rust_quality_pack:fmt_after_edit",
  "session_id": "ses_…",
  "turn_id": "trn_…",
  "org_id": "org_…",
  "agent_id": "agt_…",
  "ts": "2026-05-28T12:34:56.789Z",
  "data": { /* event-specific payload, see below */ }
}
```

Event-specific `data`:

| Event | `data` shape |
|---|---|
| `session_start` | `{ "agent_id": "agt_…" }` |
| `user_prompt_submit` | `{ "message": "...", "attachments": [...] }` |
| `pre_tool_use` | `{ "tool_name": "bash", "tool_call_id": "call_…", "arguments": { … } }` |
| `post_tool_use` | `{ "tool_name": "bash", "tool_call_id": "call_…", "arguments": { … }, "result": { … }, "error": null, "success": true }` |
| `turn_end` | `{ "success": true, "tool_calls": [...] }` |
| `session_end` | `{ "reason": "archived" }` |

### Output

The runtime parses stdout in this order:

1. If stdout is empty: equivalent to `{"decision": "allow"}`. Exit code 0 →
   allow; non-zero → block (Git-hook fallback). The body of stderr is
   surfaced as the block reason if non-empty, otherwise a generic message.
2. If stdout starts with `{`, the runtime parses it as JSON:

```json
{
  "decision": "allow" | "mutate" | "block",
  "reason": "string shown in audit/UI",
  "user_message": "string surfaced to the user when blocking",
  "patch": { /* event-specific mutation, see below */ }
}
```

3. Anything else → executor error → handled per `on_error` policy.

`patch` semantics:

| Event | `patch` shape |
|---|---|
| `pre_tool_use` | `{ "arguments": { … merged into ToolCall.arguments } }` |
| `post_tool_use` | `{ "result": …, "error": …, "additional_context": "..." }` (each optional) |
| `user_prompt_submit` | `{ "message": "rewritten user message" }` |
| Others | `patch` is ignored |

`additional_context` on `post_tool_use` is appended to the tool result as a
`hook_context` field so the model sees the hook's commentary.

### Limits

Enforced centrally by `HookAdapterBuilder`, capability authors cannot opt out:

- **Timeout**: default 5 s, configurable up to 30 s.
- **Output size**: 64 KiB total stdout + stderr (reuses the
  `OutputHardLimitHook` ceiling). Exceeding the limit is treated as an
  executor error.
- **Exec environment**: runs through `bashkit_shell` against the session VFS.
  No host shell. Inherits the session's egress policy.
- **stderr** is captured into the audit log only; never surfaced to the
  model or user unless `decision == "block"` with no `reason`.

### `on_error`

Each hook entry declares what happens when the executor fails (non-JSON
output, timeout, sandbox error, runaway size):

- `block` — treat as `{"decision": "block", "reason": "hook failed"}`.
- `allow` — log + continue.
- `warn` — log + emit a `hook.warning` event + continue.

Default is `warn`. `pre_tool_use` hooks that are security-critical should set
`block` explicitly.

## Matcher

```json
"matcher": {
  "tool_name": "edit_file",
  "tool_name_glob": "bash|edit_file",
  "args_jsonpath": "$.command",
  "match_regex": "^rm -rf",
  "deny_regex": null
}
```

- `tool_name` exact or `tool_name_glob` (basic alternation `a|b|c` and
  trailing `*` only — full glob is overkill).
- `args_jsonpath` extracts a value from `ToolCall.arguments`; `match_regex`
  is matched against it. If `match_regex` is absent the matcher fires on
  any non-empty extraction.
- `deny_regex` is the inverse: hook fires when the extracted value
  matches the regex (useful for "block any `rm -rf` command").
- An empty `matcher` matches every event of the declared kind.
- Matchers are ignored for events that have no tool context (`session_*`,
  `user_prompt_submit`, `turn_end`) — set them and they're rejected at
  config validation.

Regex flavor: `regex` crate (Rust). No look-around, no backreferences.
Total regex compile time is bounded.

## `UserHookSpec`

The on-the-wire / in-config shape:

```jsonc
{
  "id": "fmt_after_edit",                    // optional, defaults to "{event}_{idx}"
  "event": "post_tool_use",
  "matcher": { "tool_name": "edit_file" },   // optional
  "executor": {
    "type": "bash",
    "command": "scripts/fmt.sh",             // run inside bashkit_shell
    "env": { "FMT_PROFILE": "ci" }           // optional, capped at 16 vars
  },
  "timeout_ms": 5000,                        // optional, 100..30_000
  "on_error": "warn",                        // optional, default "warn"
  "description": "Run formatter after edit_file"
}
```

In Rust:

```rust
pub struct UserHookSpec {
    pub id: HookId,
    pub event: HookEvent,
    pub matcher: HookMatcher,
    pub executor: ExecutorSpec,
    pub timeout_ms: u32,
    pub on_error: OnError,
    pub description: Option<String>,
    pub source: HookSource,                  // who contributed this
}
```

`HookId` is `{capability_id}:{name}` for capability-contributed hooks and
`user:{name}` for user-config entries. IDs are surfaced in audit events
and in `disabled_contributions`.

## Composition

Hooks chain in this order:

1. All capabilities resolved in dependency-then-position order (the same
   order that already governs `tool_definition_hooks` etc.).
2. Within each capability, the order in `user_hooks()` return.

Per-event semantics:

- **`pre_tool_use`**: chain runs sequentially. Each hook sees the previous
  hook's mutated `ToolCall`. The first `block` aborts the chain and the
  tool returns an error result. Mutations from earlier hooks survive even
  if a later hook in the chain blocks.
- **`post_tool_use`**: chain runs sequentially on `&mut ToolResult`,
  matching the existing `PostToolExecHook` contract. No blocking.
- **`user_prompt_submit`**: chain runs sequentially on `&mut message`.
  First `block` rejects the turn.
- **Lifecycle events** (`session_start`, `session_end`, `turn_end`): each
  hook runs independently; failures are logged. Order is preserved for
  reproducibility but not semantically meaningful.

Disabling capability-contributed hooks is done by the user via the
`user_hooks` capability config:

```json
{
  "capabilities": [
    { "ref": "rust_quality_pack" },
    { "ref": "user_hooks", "config": {
      "disabled_contributions": ["rust_quality_pack:fmt_after_edit"]
    } }
  ]
}
```

Disabled IDs are removed from the chain at `HookAdapterBuilder` time.

## Capability contribution

The `Capability` trait gains one method, defaulted empty:

```rust
fn user_hooks(&self) -> Vec<UserHookSpec> { vec![] }
```

Capabilities return *specs*, not built adapters. The core
`HookAdapterBuilder` validates each spec against global limits, resolves the
`ExecutorSpec` to an `Arc<dyn HookExecutor>` via the executor registry, and
constructs the per-event adapter. The result is grouped by event and merged
into the existing `Vec<Arc<dyn …Hook>>` chains the runtime already iterates.

**Deferred:** declarative-capability hook bundles. The plan is for
`DeclarativeCapabilityDefinition` to gain an optional `user_hooks` field
carrying the same `UserHookSpec` schema, with the capability-write API
forcing `RiskLevel::High` whenever the array is non-empty so admin
assignment remains the trust gate. The contract lives here so the
auto-elevation rule lands with the feature; the field is *not* present
on `DeclarativeCapabilityDefinition` today, and any `user_hooks` entries
on a declarative POST body are silently ignored by serde. Tracking via
the Follow-ups list in the original PR.

## `user_hooks` capability

Built-in capability. ID: `user_hooks`. Risk: `High`. Carries no tools.

Config schema (validated at write):

```json
{
  "hooks": [ UserHookSpec, … ],
  "disabled_contributions": [ "cap_id:hook_name", … ]
}
```

Empty config is legal — the capability is then a no-op pass-through that
exists only to apply `disabled_contributions` to other capabilities' hooks.

## Audit + observability

**Delivered (v1):** hook decisions and errors are recorded via `tracing`.
Blocks, ignored post-hook blocks, mutations, and each `on_error`
outcome are logged with the resolved `hook_id` and the `tool_call_id`. Muted
contributions (via `disabled_contributions`) are logged at the
`HookAdapterBuilder` (`finalize_hook_specs`) boundary.

**Planned (deferred):** structured events the audit log persists and that
plug into the existing observation pipeline (Braintrust, OTel) the same way
tool events do:

- `hook.invoked { hook_id, event, session_id, tool_call_id?, matched: true|false }`
- `hook.completed { hook_id, decision, duration_ms, stdout_bytes, exit_code }`
- `hook.blocked { hook_id, reason, user_message? }` when `decision == block`.
- `hook.warning { hook_id, message }` when `on_error == warn` fires.

These structured events are **not emitted yet** — wiring them through the
`EventEmitter` lands alongside the audit-log integration noted in the open
questions below.

## Security

Threat model entries added to `knowledge/security/threat-model.md` under `TM-HOOK`:

- **TM-HOOK-001 — Hook-as-injection-amplifier**: a model-controlled file
  (`scripts/fmt.sh`) lets a prompt-injected agent influence hook behavior.
  Mitigation: hooks run in `bashkit_shell`, same FS sandbox as `bash`; the
  hook script is loaded by path from session VFS; running scripts that
  live in agent-writable paths is admin's choice.
- **TM-HOOK-002 — Hook-as-exfil-channel**: a hook command makes outbound
  network calls to leak session state. Mitigation: bashkit's
  `http_client` feature is compiled out for this build (see TM-BASH-003),
  so the interpreter has no built-in network surface.
- **TM-HOOK-003 — Stdout poisoning**: a long-running hook fills stdout
  with bogus JSON to deny tool execution. Mitigation: 64 KiB output cap
  + 30 s hard timeout + `on_error` policy.
- **TM-HOOK-004 — Privilege escalation via capability contribution**: a
  built-in capability bundles hooks that exfiltrate or block. Mitigation:
  the built-in `user_hooks` capability is permanently `High` and gated on
  admin assignment via `check_high_risk_caps`. (Declarative
  capability-contributed hook bundles are deferred — see the matching
  TM-HOOK-006 entry.)
- **TM-HOOK-006 — Future risk**: when declarative `user_hooks` lands, the
  capability-write API must auto-elevate to `High` whenever the array is
  non-empty. Tracked here so the contract lands with the feature.

## Extension points

Designed-in, not built v1:

- `ExecutorSpec::Webhook { url, hmac_secret_ref }` — HMAC-signed POST.
- `ExecutorSpec::Wasm { module_ref }` — wasm component executing the same
  JSON-in/JSON-out contract.
- `ExecutorSpec::Blueprint { id, config }` — invoke an agent blueprint
  as a hook (LLM-as-guardrail).
- `pre_model_call` / `post_model_call` / `subagent_stop` / `pre_compact`
  / `notification` events.
- Per-org policy that downgrades the `user_hooks` capability's risk for
  trusted operators.

Each addition is a new `HookEvent` variant + a new `ExecutorSpec` variant.
No breaking changes to existing user configs.

## Implementation index

| Concern | Location |
|---|---|
| `HookEvent`, `HookMatcher`, `HookOutcome`, `UserHookSpec`, `ExecutorSpec`, `OnError`, `HookId`, `HookSource` | `crates/core/src/user_hook_types.rs` |
| `HookExecutor` trait + `BashHookExecutor` | `crates/core/src/hook_executor.rs` |
| `PreToolUseHook` trait | `crates/core/src/tool_hooks.rs` (alongside existing hooks) |
| `SessionLifecycleHook`, `TurnLifecycleHook` traits | `crates/core/src/lifecycle_hooks.rs` — defines both traits plus `BashLifecycleHook` |
| `HookAdapterBuilder` (spec → adapter) | `crates/core/src/hook_adapter.rs` |
| `Capability::user_hooks()` default + collection extension | `crates/core/src/capabilities/mod.rs` |
| `user_hooks` capability | `crates/platform/src/capabilities/user_hooks.rs` |
| `pre_tool_use` wire-in | `crates/engine/src/execution/act.rs::execute_single_tool` |
| `post_tool_use` wire-in | existing `PostToolExecHook` chain |

## Open questions (deferred, tracked)

- Should `disabled_contributions` accept glob patterns (e.g. `rust_quality_pack:*`)?
  Defer until someone hits the need.
- Should `pre_tool_use` chains support short-circuit `allow_skip_remaining`
  for the matcher-precedence pattern? Defer; chain semantics are simpler.
- Should hook stdout/stderr persist into session VFS under
  `/.hooks/{hook_id}/{call_id}.log` like exec output? Probably yes, but
  separate change once the audit-log integration lands.
