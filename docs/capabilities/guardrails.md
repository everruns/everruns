---
title: Guardrails
description: Config-driven checks that constrain agent behavior, inspecting model output and tool activity, then blocking or logging when content matches a rule.
sidebar:
  order: 94
---

# Guardrails

| | |
|---|---|
| **ID** | `guardrails` |
| **Category** | Safety |
| **Tools** | None |
| **Dependencies** | Utility LLM (only for model-backed checks) |
| **Risk** | Low |

Guardrails are checks that constrain what an agent does. Where most [capabilities](/capabilities/) *grant* an ability, a guardrail *restricts* one: it inspects model output and tool activity, then **blocks** or **logs** when content matches a rule.

Guardrails are opt-in. An agent with no guardrails is a fully supported configuration, there is no org-mandated enforcement layer. A [harness](/features/harnesses/) can attach guardrail capabilities as soft defaults that flow to every agent built on it, and an author can still remove them. This is the platform stance: guardrails are a default posture, not a cage.

The `guardrails` capability holds no rules of its own. Its per-agent config is a declarative list of checks plus a mode; the capability compiles that config and contributes the matching runtime hooks. An empty config (or the capability being absent) contributes nothing, with zero added latency.

## Concepts

A **check** binds a **rule** to a **stage** with an **on-fail action**.

### Stages

| Stage | What it sees |
|-------|--------------|
| `output` | Streamed assistant text |
| `tool_use` | A tool call before it executes, the tool name and serialized arguments |
| `tool_output` | A tool result before it enters model context |

`tool_output` is the trust boundary for untrusted external content (web pages, MCP responses); indirect-injection and secret-leakage checks belong there.

### Rules

| Rule (`type`) | What it matches | Valid stages | Execution |
|---------------|-----------------|--------------|-----------|
| `regex` | Any of the patterns matches the stage text | all | in-process, sync |
| `blocklist` | Any word/phrase appears as a substring (case-insensitive by default) | all | in-process, sync |
| `tool_pattern` | The tool name matches a `*`-wildcard glob | `tool_use` only | in-process, sync |
| `llm_judge` | A natural-language policy, evaluated by the utility LLM | `tool_use`, `tool_output` | async, model-backed |
| `mcp` | Decision delegated to an external guardrail served over scoped MCP | `tool_use`, `tool_output` | async, off-platform |
| `moderation` | The finalized message scored by the utility LLM as a content classifier | `output` only | async, model-backed |

Deterministic rules (`regex`, `blocklist`, `tool_pattern`) run in the streaming and per-tool-call hot path, linear-time, no I/O, with hard limits on check count, entries, and lengths so an authored pattern can never wedge a worker. Model-backed and MCP rules run only in the async hook path (and, for `output`, on a post-generation end-of-message boundary), never on the sync hot path.

### On-fail

- `block`, suppresses the matched content: an `output`/`tool_output` block replaces the content with a notice; a `tool_use` block refuses the call and feeds the reason back to the model, which can self-correct. The model's original tokens are never persisted.
- `log`, records the hit and continues.

An optional per-check `replacement` customizes the block notice or user-facing refusal message.

### Mode: active vs. advisory

A config-level `mode` is `active` (default) or `advisory`. **Advisory downgrades every hit to `log`**: checks run and are recorded, but nothing is blocked. Advisory is how you tune a guardrail against false positives before enforcing it. Mode is per attachment, so the same catalog entry can be advisory on one agent and active on another.

## Config shape

Config is a `GuardrailsConfig` stored under the `guardrails` capability in the agent's config. Field names are `snake_case`; each check names its rule with a `type` tag alongside the shared `stage` / `on_fail` / `replacement` fields:

```json
{
  "mode": "active",
  "checks": [
    {
      "id": "no-secrets-in-output",
      "stage": "output",
      "on_fail": "block",
      "replacement": "[Response withheld: appears to contain a credential.]",
      "type": "regex",
      "patterns": ["AKIA[0-9A-Z]{16}", "ghp_[A-Za-z0-9]{36}"]
    },
    {
      "id": "no-shell",
      "stage": "tool_use",
      "on_fail": "block",
      "type": "tool_pattern",
      "tools": ["bash*", "*exec*"]
    }
  ]
}
```

The `id` is optional but recommended, it is surfaced in reason codes and logs.

## Data egress and failure behavior

- **Deterministic checks** (`regex`, `blocklist`, `tool_pattern`) run entirely in-process; no data leaves the platform.
- **`llm_judge` and `moderation`** send a bounded content excerpt to your org's *own* configured utility LLM, the same provider the agent already uses, not a new third party.
- **`mcp`** sends a bounded content excerpt to an external, operator-configured MCP guardrail endpoint. Tenant scoping is enforced by the host's per-session scoped-MCP resolver, so a config can only reach servers scoped to its own session/org.

Every async check is bounded (10 s timeout, at most 4 calls per invocation) and **fails open**: a timeout, error, or unparseable verdict defaults to `allow`. A guardrail outage, or a hostile MCP endpoint, can only ever *allow*, never make execution more permissive than the no-guardrail baseline in a way that blocks a healthy turn. Model-backed checks flow through utility-LLM accounting, not the session model budget.

## Tuning: dry-run and advisory

Two surfaces let you tune checks before enforcing them:

- **`POST /v1/capabilities/guardrails/dry-run`** evaluates a config against sample text for a given stage, with no session and nothing persisted. It returns the triggered checks (id, rule type, effective action, reason code, matched excerpt) and whether the content would be blocked. It runs only deterministic checks, it never makes a network call, so it is the fast false-positive tuning loop for `regex`/`blocklist`/`tool_pattern`.
- **Advisory mode** runs the full set (including model-backed checks) against real traffic in `log`-only form, so you can review what *would* have been blocked before switching to `active`.

## The gallery: ready-made presets

Rather than authoring checks from scratch, list the **guardrail gallery**: a read-only catalogue of adoptable presets:

```
GET /v1/capabilities/guardrails/examples
```

Each listing carries a full `config` plus trust metadata so a picker can show what a preset does before adoption:

| Field | Meaning |
|-------|---------|
| `check_types` | The rule-type composition (e.g. `["regex"]`, `["llm_judge"]`) |
| `stages` | Which stages the preset's checks run in |
| `data_egress` | `none` for deterministic presets; `utility_llm` when a preset contains a model-backed check |

`data_egress` is **derived from the check types**, not hand-authored, so it stays correct as presets mix deterministic and model-backed checks. Adoption is client-side config composition: drop a preset's `config` into the agent's `guardrails` capability config (merging or replacing checks). There is no new persisted resource and no import endpoint. Noisy presets (PII, prompt-injection heuristics) ship `log`-only so they are safe to adopt active and tune before switching individual checks to `block`.

Shipped presets include secret detection, a model-backed secret-leak judge, PII detection, a profanity starter, dangerous-shell blocking, shell-access blocking, and prompt-injection heuristics.

### Worked example: deterministic vs. model-backed secret guardrails

Two presets guard the same risk, a secret reaching output or leaving through a tool, by complementary means.

**`secret-detection`** matches known credential *formats* by pattern. It is in-process and reports no egress:

```json
{
  "name": "secret-detection",
  "check_types": ["regex"],
  "stages": ["output", "tool_output"],
  "data_egress": "none"
}
```

It blocks well-known formats (AWS, GitHub, Slack, Google keys, PEM private keys) in model output and in tool results before they reach context. High-precision, safe to run active.

**`secret-leak-judge`** catches secrets by *intent*, including an opaque value whose form is unknown at config time (e.g. one freshly read from a secrets manager), which a regex structurally cannot see:

```json
{
  "name": "secret-leak-judge",
  "check_types": ["llm_judge"],
  "stages": ["tool_use", "tool_output"],
  "data_egress": "utility_llm"
}
```

Its `llm_judge` policy blocks a tool call (or result) that would print, echo, log, or transmit secret material in cleartext, while allowing comparisons that reveal only a hash, fingerprint, or redacted form. Because it sits on `tool_use` with `on_fail: block`, a blocked call is **recoverable**: the refusal reason is fed back and the model self-corrects to a safe form (comparing a hash instead of printing the secret). It is model-backed, so `data_egress` is `utility_llm`, adopters are correctly warned that a bounded excerpt leaves the generating path for evaluation. Run it advisory first to tune false positives.

The two are meant to be layered: pattern-matching for known formats, the judge for everything else.

## Reason codes

Every block or log carries a stable `guardrail.<rule_type>` code (e.g. `guardrail.regex`, `guardrail.llm_judge`, `guardrail.moderation`). Clients localize copy from the code rather than the human-readable text. (The separate [Prompt Canary Guardrail](/capabilities/prompt-canary-guardrail/) capability uses its own `system_prompt_leak` code.)

## Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/v1/capabilities/guardrails/examples` | List adoptable gallery presets with trust metadata |
| `POST` | `/v1/capabilities/guardrails/dry-run` | Evaluate a config against sample text; nothing persisted |

Both are gated by the same `capability.view` policy as other capability reads.

## Related

- [Prompt Canary Guardrail](/capabilities/prompt-canary-guardrail/), a narrow streaming-output guardrail for naive system-prompt leakage.
- [Agent Checks](/features/agent-checks/), advisory, config-*time* review of an agent's setup. Distinct from guardrails, which enforce at *runtime*.
