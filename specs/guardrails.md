# Guardrails Specification

## Abstract

Guardrails are checks that constrain agent behavior — inspecting model output
and tool activity, then blocking or logging when content matches a rule.
Unlike most [capabilities](capabilities.md), which *grant* abilities, a
guardrail *restricts* them. Guardrails are modeled as capabilities rather than
a separate top-level entity: they reuse the registry, per-agent config schema,
validation, risk gating, and harness/agent/session composition that
capabilities already provide, and they attach to the runtime through the
existing interception seams (streaming output guardrails, pre/post tool
hooks).

This is a deliberate product stance: everruns is a platform for building *any*
agent. Guardrails are opt-in. An agent author may build an agent with no
guardrails, and that is a supported configuration — there is no org-mandated
enforcement layer in this phase. A harness can bundle guardrail capabilities as
soft defaults that flow to every agent built on it, and the author can still
remove them.

## Concepts

A guardrail is a capability whose `is_guardrail()` marker is `true`. The marker
carries no runtime semantics; it exists for UI grouping (a "Guardrails" section
in the agent builder) and catalog filtering. `is_guardrail` is surfaced on
`CapabilityInfo`.

Two guardrail capabilities ship today:

- `prompt_canary_guardrail` — detects the model echoing the first sentence of
  its own system prompt and replaces the response. Narrow, no config. Predates
  this spec; now flagged `is_guardrail`.
- `guardrails` — the general, config-driven deterministic check engine
  described below.

### The `guardrails` capability

The `guardrails` capability holds no rules of its own. Its per-agent config
(`AgentCapabilityConfig.config`) is a declarative `GuardrailsConfig`: a list of
typed checks plus a mode. The capability compiles that config and contributes
the matching runtime hooks. An empty config (or the capability being absent)
contributes nothing — an agent without guardrail checks runs exactly as it did
before this feature existed, with zero added latency. The config schema is
exposed via `config_schema()` so clients render a generic editor with no
hard-coded knowledge of check types.

The check engine is `everruns_core::guardrail_checks`; the capability wiring is
`crates/core/src/capabilities/guardrails.rs`. See those for exact field names
and limits rather than duplicating them here.

## Checks

A check binds a **rule** to a **stage** with an **on-fail action**:

- **Stages** — `output` (streamed assistant text), `tool_use` (a tool call
  before execution; rules see the tool name and serialized arguments), and
  `tool_output` (a tool result before it enters model context). The
  `tool_output` stage is the trust boundary for untrusted external content
  (web pages, MCP responses) and is where indirect-injection and
  secret-leakage checks belong.
- **Rules** (deterministic only in this phase) — `regex` (any pattern matches),
  `blocklist` (any word/phrase is a substring; case-insensitive by default),
  and `tool_pattern` (the tool name matches a `*`-wildcard glob; valid only on
  the `tool_use` stage).
- **On-fail** — `block` or `log`. `block` suppresses the matched content
  (replacing output/tool-output with a notice, or refusing the tool call);
  `log` records the hit and continues. Optional per-check `replacement` text
  customizes the block notice / user-facing refusal message.

### Mode

A config-level `mode` is either `active` (default) or `advisory`. Advisory
downgrades every hit's effective action to `log`: checks run and are recorded
but nothing is blocked or replaced. Advisory is how a guardrail is tuned
against false positives before being made active. Mode is per *attachment* —
the same guardrail catalog entry can be advisory on one agent and active on
another, because it lives in each agent's capability config.

### Determinism and the streaming hot path

Phase-1 checks are deterministic and run in the streaming output path and the
per-tool-call path, so every rule must evaluate in linear time with no I/O. The
`regex` crate guarantees linear-time matching (no catastrophic backtracking),
and the engine enforces hard limits on check count, entries per check, entry
length, and replacement length (TM-DOS, TM-API input validation). User-authored
patterns therefore cannot wedge a worker. Model-based checks (classifiers,
LLM-as-judge) are a planned later phase and will never share this synchronous
code path — they run at message-end or parallel-to-LLM with cancellation.

## Runtime integration

The capability compiles its config once and contributes hooks only for stages
that have at least one check:

- **`output`** → an `OutputGuardrail` (see [output_guardrail](../crates/core/src/output_guardrail.rs)).
  Armed per assistant-message stream; evaluated against the accumulated text on
  each delta so matches spanning delta boundaries are caught. A blocking hit
  aborts the stream and emits `output.message.replaced`; the original tokens
  are never persisted. Advisory hits are logged once per stream (not re-logged
  on every subsequent delta).
- **`tool_use`** → a `PreToolUseHook`. A blocking hit refuses the tool call and
  feeds the reason back to the model (which can self-correct); sibling calls in
  the batch are unaffected. Capability-contributed pre-hooks, including this
  one, run before user-hook (`PreToolUse`) specs; the first block wins.
- **`tool_output`** → a `PostToolExecHook`. A blocking hit replaces the tool
  result with the notice and drops the error/images/raw payloads so the
  original content never reaches model context. Runs before the infrastructure
  output-size-limit hook.

Invalid persisted config (only possible if persisted before validation
existed) is logged and treated as no checks: guardrails must never take down
the turn pipeline.

## Dry run

`POST /v1/capabilities/guardrails/dry-run` evaluates a `GuardrailsConfig`
against sample text for a given stage, with no session and nothing persisted.
It returns the triggered checks (id, rule type, effective action, reason code,
matched excerpt) and whether the content would be blocked. This is the
false-positive tuning surface and pairs with advisory mode: authors iterate on
patterns against real samples before attaching a guardrail to an agent. Input
text is size-bounded. Gated by the same `capability.view` policy as other
capability reads.

## Gallery

The guardrail gallery is a read-only catalogue of ready-made `GuardrailsConfig`
presets (secret detection, PII detection, a profanity starter, dangerous-shell
blocking, shell-access blocking, prompt-injection heuristics). It mirrors the
harness-examples pattern: presets live in code
(`everruns_core::guardrail_gallery`), not the DB, and are served read-only via
`GET /v1/capabilities/guardrails/examples` (gated by `capability.view`).

Adoption is client-side config composition: each listing carries a full
`config`, and a client drops it into an agent's `guardrails` capability config
(merging or replacing checks). There is no new persisted resource and no
import endpoint — guardrail configs already live in agent capability config.

Each listing carries trust metadata so a picker can show what a preset does
before adoption: `check_types` (the rule-type composition), `stages`, and
`data_egress`. Deterministic presets report `data_egress = none` (everything
runs in-process); model-based and MCP-served presets will report other egress
markers when those check types land, derived from the check types rather than
hand-authored. Presets that are inherently noisy (PII, prompt-injection
heuristics) ship `log`-only so they are safe to adopt active and tuned before
switching individual checks to `block`.

## Composition and removal

Guardrail capabilities compose through the standard
[`AgentConfigOverlay`](capabilities.md) fold: a harness can attach guardrail
capabilities that every agent on it inherits, an agent can add its own, and a
session can add more. Because guardrails are capabilities, the existing overlay
rule applies — an overlay layer overrides a base layer's config for the same
capability ref. There is no "cannot be removed" enforcement: removing a
guardrail capability from an agent removes its checks for that agent's
subsequent sessions. This matches the platform stance that guardrails are a
default posture, not a cage.

If a future requirement calls for org-mandated guardrails, the additive path is
an org layer at the bottom of the overlay fold that unions in capability refs —
an extension of the existing merge, not a rework. It is intentionally not built
in this phase.

## Reason codes

Blocking and logging both carry a stable machine-readable reason code,
`guardrail.<rule_type>` (e.g. `guardrail.blocklist`). Clients localize copy
from the code rather than the human text. The `prompt_canary_guardrail`
continues to use its own `system_prompt_leak` code.

## Security

- Deterministic checks run in-process with no external network access.
  Model-based and external (MCP-served) checks are future phases and will carry
  the appropriate `risk_level` and admin-assignment gate when added.
- The dry-run endpoint performs pure computation, persists nothing, and bounds
  input size (TM-DOS).
- Compile-time limits bound regex/blocklist cost (TM-API, TM-DOS).
- See [threat-model.md](threat-model.md) `TM-LLM` (output handling),
  `TM-TOOL` (tool-call interception), and `TM-AGENT` (capability assignment).

## Future phases (not implemented)

- Model-backed checks: PII (NER), profanity/toxicity, prompt-injection
  classifiers; provider moderation APIs first, local models later.
- `llm_judge` check type: natural-language policy prompt evaluated by a fast
  model at message-end or parallel-to-input; cost flows through budgeting.
- `mcp` check type: a third-party guardrail served as an external endpoint over
  existing scoped-MCP auth.
