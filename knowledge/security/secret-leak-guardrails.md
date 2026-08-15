---
type: Specification
title: "Secret-leak guardrails"
description: "Defense-in-depth design for model-backed secret detection and deterministic known-value redaction."
tags:
  - everruns
  - security
  - guardrails
  - secrets
---

# Secret-leak guardrails

Status: active design. The model-backed `llm_judge` preset ships. A dedicated
secret-leak classifier, output-stage semantic checks, and deterministic
known-value redaction remain proposed.

## The scenario

An agent is rotating an AWS Secrets Manager `clientSecret`. Mid-task it wants to
verify propagation. Two guardrail behaviors are visible:

1. A tool call that would **echo part of the live secret** (a compare that
   printed secret material) is **blocked**, and the refusal reason is fed back.
2. The model **self-corrects** to a hash comparison — "no secret material in the
   output at all" — and proceeds.

## What kind of guardrail this is

The guardrail in the screenshot is almost certainly **model-backed** — a
classifier or LLM judge — **not** a match against a pre-known secret value. It
did not need the `clientSecret` enrolled ahead of time; it recognized
*semantically* that the command was about to print credential material, then
recognized the hash-only retry as safe. Generic secret detection with no prior
knowledge of the value is exactly what a judge/classifier is for, and what a
substring or regex matcher structurally cannot do.

This reframes the everruns answer. There are two families of check, and the
faithful match is the second:

- **Deterministic** (`regex`, `blocklist`, `tool_pattern`) — fast, in-process,
  but can only catch *known formats* or *known values*.
- **Model-backed** (`llm_judge`, `moderation`, `mcp`) — dynamic, no prior
  knowledge of the value, judges intent/shape. This is the screenshot's class.

## Verdict

**Yes — and the model-backed primitive already ships.** everruns' `llm_judge`
check on the `tool_use` stage reproduces the screenshot end-to-end today:
dynamic, value-agnostic, block-and-feed-reason-back so the model self-corrects.
Full parity needs one thing everruns hasn't enabled yet (judge on the `output`
stage, EVE-572) and benefits from one new thing (a dedicated, cheaper
secret-leak classifier). A deterministic known-value redactor is a valuable
*complementary* layer, not the primary mechanism.

## The faithful match: `llm_judge` on `tool_use` (already shipped)

Grounded in `crates/builtins/src/guardrails.rs` and
`crates/core/src/guardrail_checks.rs`:

- `GuardrailRule::LlmJudge { prompt }` sends the stage name, tool name, and a
  bounded content excerpt (2 000 bytes, UTF-8-safe) to the utility LLM with
  `JUDGE_SYSTEM_PROMPT` ("You are a guardrail policy evaluator…"), and parses a
  `{"verdict":"allow"}` / `{"verdict":"block","reason":"…"}` response
  (`run_judge_check`).
- On `tool_use` it runs in the **pre-tool hook**; a `block` verdict refuses the
  call and, per [`guardrails.md`](../execution/guardrails.md) §Runtime integration, "feeds the reason
  back to the model (which can self-correct)." That is exactly the screenshot:
  block the secret-echoing compare → model retries as a hash compare.
- It is **value-agnostic**: the policy prompt describes the *class* of thing to
  block; no secret needs to be enrolled.

Concretely, the config is a single check whose prompt is the policy:

```
Block any tool call whose arguments would print, echo, log, diff, or transmit
secret or credential material in cleartext — API keys, tokens, passwords,
private keys, connection strings, or values read from a secrets manager. Allow
comparisons that only reveal a hash, length, or redacted form. Allow reads that
store the secret without displaying it.
```

The same rule on `tool_output` catches a fetched file/result that carries a
secret before it reaches context.

### Known operational limits (from the shipped implementation)

- **Async, hook-path only.** `llm_judge` never runs in the streaming hot path.
  Runs in pre/post-tool hooks with a 10 s timeout (`JUDGE_TIMEOUT`).
- **Fail-open.** Timeout, LLM error, or unparseable verdict ⇒ `allow`, so a judge
  outage never wedges a turn. A secret-leak guardrail that fails open is a real
  security limitation to state plainly — advisory rollout, plus the deterministic
  layer below as a fail-closed backstop for *known* values.
- **Cost + latency.** Each judged tool call is a utility-LLM round trip (low
  reasoning effort), capped at `MAX_JUDGE_CALLS_PER_INVOCATION` (4) per
  invocation, accounted through utility-LLM billing, not the session budget.
- **`output` stage not yet enabled.** Today only `moderation` runs on `output`
  (the end-of-message seam, EVE-573); `llm_judge`/`mcp` on `output` is tracked in
  EVE-572. So the prose-echo case ("model writes the secret in its answer") needs
  either EVE-572 or a `moderation`-style classifier on `output`.

## A better fit for the hot/common case: a dedicated secret-leak classifier

A general LLM judge on *every* tool call is heavy. The screenshot's guardrail is
more plausibly a **narrow, cheap classifier** (as Claude Code likely uses),
analogous to everruns' existing `moderation` check, which already runs the
utility LLM "acting as a content classifier" with a fixed
`MODERATION_SYSTEM_PROMPT` and a category/threshold verdict. The natural
addition:

- A `secret_leak` classifier check (a sibling of `moderation`) with a fixed
  system prompt tuned for one job — "does this content reveal secret/credential
  material in cleartext?" — returning a score/threshold verdict. Cheaper and more
  consistent than an open-ended judge prompt, and enable-able on `output`,
  `tool_use`, and `tool_output`. This mirrors the spec's own "Future phases":
  PII (NER) and prompt-injection classifiers as dedicated model-backed checks.

## The complementary deterministic layer (fail-closed, exact, free)

Model-backed checks fail open and cost a round trip. For secrets the platform
*does* know — the old/new `clientSecret` during a rotation, MCP OAuth tokens,
user-provided API keys — a deterministic **known-value redactor** is a cheap,
exact, in-process backstop that fails *closed*:

- New `GuardrailRule::SecretValues { source, min_len, normalize }`, deterministic,
  valid on all three stages, matching via Aho-Corasick (linear, no backtracking,
  same DoS posture as the existing blocklist/regex matchers).
- Value source: `session_storage`'s `secret_store` — the encrypted, session-scoped
  namespaced-secret store (`ns` table, AES-256-GCM per [`encryption.md`](encryption.md)),
  which already holds MCP OAuth tokens (`mcp_oauth_ns_name`), sandbox state, and
  user API keys. Optionally auto-enroll high-entropy values seen at `tool_output`.
- New **`redact`** action (mask the matched span) alongside `block`/`log`, so the
  useful part of a command/answer survives while the secret is masked.
- One architectural change: `evaluate()` is currently pure over config-time data.
  `SecretValues` needs a session-scoped secret handle, wired into the hooks the
  same way `llm_judge`/`mcp` already receive their session collaborators.

## Recommended architecture: defense in depth

The two layers cover each other's weaknesses:

| Layer | Catches | Fails | Cost |
|---|---|---|---|
| `llm_judge` / `secret_leak` classifier (model-backed) | *unknown* secrets, by shape/intent — the screenshot | open | utility-LLM round trip |
| `SecretValues` redactor (deterministic) | *known/enrolled* values, exactly | closed | in-process, ~free |

Run both: the classifier is the dynamic front line (reproduces the screenshot),
the deterministic redactor is the exact, fail-closed net for values the platform
already holds. Advisory mode + dry-run
(`POST /v1/capabilities/guardrails/dry-run`) tune the classifier's false
positives before it goes active.

## Honest limits

- **Model-backed checks fail open.** A judge/classifier outage degrades to no
  protection. The deterministic layer is the fail-closed complement, but only for
  known values.
- **Encoding evades the deterministic layer.** base64/URL-encoding/hashing a
  secret defeats substring matching; `normalize` covers whitespace/case only. The
  classifier is more robust here but still probabilistic. The model's hash-compare
  self-correction is the *right* pattern precisely because it emits no value in
  any encoding.
- **Self-correction is not guaranteed.** Feeding the reason back usually yields a
  compliant retry; per-turn tool-call limits bound loops.
- **`output`-stage semantic checks are gated on EVE-572** (or a new classifier);
  today only `moderation` runs on `output`.

## Phasing

1. **Shipped:** the `secret-leak-judge` gallery preset — an `llm_judge` policy on
   `tool_use` + `tool_output` (`guardrail_gallery.rs`). Reproduces the screenshot
   for tool calls today; the gallery's `data_egress` is now derived from check
   types (`utility_llm` for model-backed presets). Run advisory-first to tune.
2. **Dedicated `secret_leak` classifier** (a `moderation` sibling), enable-able on
   all three stages — cheaper, more consistent, and covers prose echo on `output`.
3. **Deterministic `SecretValues` redactor** + `redact` action, sourced from
   `secret_store`, as the fail-closed exact layer.
4. **EVE-572** (`llm_judge`/`mcp` on `output`) if an open-ended judge is wanted on
   prose output rather than the fixed classifier.

Phase 1 is configuration-only and reproduces the screenshot's tool-call behavior
immediately; phases 2–3 add coverage and a fail-closed backstop.
