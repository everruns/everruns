# Proposal: Known-value secret-leak guardrail

Status: draft design note (pre-spec). Answers "can we build a guardrail that
blocks an agent from echoing a secret it holds in context, and how?" On
acceptance this becomes a new rule variant in `everruns_core::guardrail_checks`,
a gallery preset, and additions to `specs/guardrails.md`.

## The scenario

An agent is rotating an AWS Secrets Manager `clientSecret`. Mid-task it wants to
verify propagation. Two guardrail behaviors are visible:

1. A tool call that would **echo part of the live secret** (a diff/compare that
   printed secret material) is **blocked**. The refusal reason is fed back to
   the model.
2. The model **self-corrects** to a hash comparison — "no secret material in the
   output at all" — and proceeds.

So "such a guardrail" is really two things: a **detector** that recognizes when
stage content contains a secret the agent is handling, and the **block →
feed-reason-back → self-correct** loop around it. The second half already exists;
the first half exists only for *well-known credential formats*, not for the
*opaque, session-specific* value in this scenario.

## Verdict

**Yes, and most of the machinery is already in place.** everruns already ships a
config-driven guardrail engine (`specs/guardrails.md`) with exactly the three
interception seams this needs, the block-and-feed-back loop, advisory mode, a
dry-run tuning surface, and a `secret-detection` gallery preset. The one missing
primitive is a **dynamic, session-scoped source of secret *values*** feeding a
deterministic redaction check. Everything else is reuse.

## What already exists (and directly covers half the ask)

Grounded in `crates/core/src/guardrail_checks.rs`,
`crates/core/src/guardrail_gallery.rs`, and `crates/core/src/output_guardrail.rs`:

- **Three stages**, which are exactly the leak surfaces: `output` (model echoes
  the secret in prose), `tool_use` (the *command about to run* embeds the
  secret — this is the screenshot's block), `tool_output` (a fetched file/result
  carries one). See `GuardrailStage`.
- **The self-correction loop is already the `tool_use` contract.** Per
  `specs/guardrails.md` §Runtime integration, a blocking `tool_use` hit "refuses
  the tool call and feeds the reason back to the model (which can self-correct)."
  The screenshot's step 2 (redo as a hash compare) is this behavior, not new work.
- **Substring matching already exists** as `GuardrailRule::Blocklist` — a
  compiled, case-optional, linear-time `haystack.contains(word)` matcher
  (`CompiledRule::Blocklist`). A known-secret redactor is a blocklist whose word
  list is *sourced at runtime* instead of at config time.
- **A `secret-detection` preset already ships** (`guardrail_gallery.rs`) with
  high-precision regexes for AWS/GitHub/Slack/Google keys and PEM headers, on
  `output` + `tool_output`, safe to run active.
- **Advisory mode + dry-run** (`POST /v1/capabilities/guardrails/dry-run`) are
  the false-positive tuning path — essential for anything heuristic.

## The gap

The screenshot's secret is an **opaque, random, session-specific** value read
from AWS SM at runtime. It has no well-known format, so the static `regex`
preset cannot see it, and `Blocklist` can't help because **the value is unknown
at agent-config time.** Two capabilities are missing:

1. **A dynamic secret source.** The guardrail must know *which concrete values*
   are "live" for this session, and that set changes per run.
2. **Value-based deterministic matching over that set**, including partial
   matches ("echoed *part* of the secret"), running on all three stages, with a
   **redact** outcome (mask the matched span) in addition to today's whole-message
   block.

Neither the sync `evaluate()` path nor any compiled rule has a runtime handle to
session state today — `evaluate()` is pure over config-time data. That is the
core thing to add.

### Where the secret values come from

everruns already has the source. `session_storage` exposes `secret_store` — an
encrypted, session-scoped namespaced-secret store (the `ns` table,
AES-256-GCM envelope encryption per `specs/encryption.md`). It already holds
agent-handled credentials: MCP OAuth tokens (`mcp_oauth_ns_name`), managed
sandbox state (`specs/session-sandbox.md`), and user-provided API keys. A
known-value guardrail draws its match set from:

- **Enrolled secrets** — values the agent (or a harness) explicitly places in
  `secret_store`. Deterministic, precise, zero false positives. This is the
  clean path for the rotation scenario: the old/new `clientSecret` is enrolled,
  and any stage content containing it is redacted.
- **Auto-enrolled secrets** — high-entropy values observed in the output of
  designated secret-reading tools (e.g. a raw `aws secretsmanager get-secret-value`
  in bash). The value transits `tool_output` first; that seam both *learns* the
  value into the session redaction set and redacts it downstream. Opt-in, because
  it is heuristic.

## Design

### New deterministic rule variant

Add to `GuardrailRule` (`guardrail_checks.rs`):

```
SecretValues {
    source: SecretSource,   // enrolled | auto_enroll | explicit(list)
    min_len: usize,         // default ~12: ignore short/low-entropy fragments
    normalize: bool,        // fold whitespace/case before matching (default true)
    partial: bool,          // also match a long contiguous run of a secret
}
```

Valid on all three stages. Effective action gains a third option alongside
`block`/`log`: **`redact`** — replace only the matched span(s) with a mask
(`«redacted»`) rather than withholding the whole message. Redaction is what
lets the *useful* part of a tool call or answer survive while the secret does
not, matching the screenshot's "no secret printed" outcome without nuking the
turn. `block` and `log` remain for callers who want them.

### Runtime wiring (the one architectural change)

Today `CompiledGuardrails::evaluate()` takes only config-time data plus a `skip`
closure. `SecretValues` needs the session's current secret set. Mirror how
`llm_judge`/`mcp` checks already receive their collaborators from context: those
are pulled out of the sync path (`judge_checks_for_stage`, `mcp_checks_for_stage`)
and run by the capability hooks with a session-scoped invoker. Add the same
shape for secrets:

- A `SecretRedactor` handle (a compiled `matcher::AhoCorasick` over the current
  secret set, rebuilt when the set changes) supplied to the output/pre-tool/
  post-tool hooks at arm time from the session's `secret_store`.
- Matching stays **deterministic and linear** — Aho-Corasick over N secrets is
  O(text length), no backtracking, same DoS posture as the existing regex/
  blocklist matchers. The `min_len` gate and an entropy floor prevent a short or
  dictionary-word secret from redacting half the transcript.
- The pure `evaluate()` stays pure for regex/blocklist/tool_pattern; `SecretValues`
  is evaluated in the hooks where the session handle is available, exactly like
  the async variants — but it is still *synchronous and deterministic*, just
  session-stateful.

### Gallery presets

- `known-secret-redaction` — `SecretValues{source: enrolled}` on all three
  stages, action `redact`, active-safe (zero false positives: it only masks
  values explicitly enrolled).
- `entropy-secret-heuristic` — a `regex`/entropy detector for unknown
  high-entropy tokens on `output`+`tool_output`, shipped **log-only** like the
  existing PII and prompt-injection presets, because generic secret detection is
  inherently noisy.

## Hard parts and honest limits

- **You can only redact what you can identify.** Enrolled/auto-enrolled known
  values are exact and safe. The fully general "detect any secret the agent
  never told us about" case is entropy heuristics — false-positive-prone, hence
  log-only first, tuned via advisory + dry-run before `block`/`redact`.
- **Encoding evades substring matching.** A secret that is base64'd, URL-encoded,
  or hashed before it hits a stage won't match the raw value. `normalize` covers
  whitespace/case; common transforms (base64, %-encoding) can be layered in, but
  this is a cat-and-mouse tail, not a guarantee. The hash-compare self-correction
  in the screenshot is the *right* pattern precisely because it never emits the
  value in any encoding.
- **Self-correction is not guaranteed.** Feeding the reason back usually yields a
  compliant retry, but a model can loop; the existing per-turn tool-call limits
  bound this. Advisory mode is the safe rollout default.
- **Auto-enrollment widens what the platform reads.** Learning values from
  `tool_output` means inspecting tool results for high-entropy strings; it must
  be opt-in and scoped to designated tools, and the learned set lives only in
  encrypted session storage (TM-LLM / TM-TOOL in `specs/threat-model.md`).

## Phasing

1. **Ship-nothing-new baseline:** the `secret-detection` regex preset already
   covers well-known formats today. Document it as the answer for formatted keys.
2. **Known-value redaction (the direct build for the screenshot):**
   `SecretValues{enrolled}` + `redact` action + the `SecretRedactor` hook wiring,
   sourced from `secret_store`. Deterministic, active-safe, no model calls.
3. **Auto-enrollment** from designated secret-reading tool outputs (opt-in).
4. **Entropy heuristic** gallery preset (log-only) for unknown secrets, plus an
   optional `llm_judge`/`moderation`-style "never reveal credential material"
   semantic backstop on `tool_use`/`tool_output` for cases substring matching
   structurally cannot catch.

Phase 2 alone reproduces the screenshot end-to-end and is a small, testable,
in-process change with zero new egress.
