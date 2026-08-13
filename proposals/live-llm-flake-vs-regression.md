# Proposal: separate "the model declined" from "our harness is broken" in the live LLM matrix

Status: draft design note (pre-spec). Answers "how do we stop model sampling
reddening `main` without giving up the only unmocked check that our provider
integration works?" On acceptance this becomes: a deterministic tool-offer
precondition asserted on every live case, a `skip_if_model_declined!` companion
to the existing `skip_if_quota!`, and a skip ledger so silent erosion is
visible.

## The problem

`main` went red on three consecutive merges on 2026-08-12. Two were live LLM
cases:

| main commit | failing case |
|---|---|
| `834a3a923` | `agent_run_basic` → `test_tool_call::case_5_openrouter_gpt4o_mini` |
| `69b1274d5` | `agent_run_basic` passed 22/22; `agent_run_with_thinking` → `test_extended_thinking::case_1_anthropic_opus5` failed |
| `5d58810b4` | Shell Tests, `curl 503` — infrastructure, unrelated |

The first case reported:

```
attempt 1/3 not acceptable (success=true, transient_transport=false, tool_calls=0, iterations=1, error=None); retrying
attempt 2/3 not acceptable (success=true, transient_transport=false, tool_calls=0, iterations=1, error=None); retrying
attempt 3/3 not acceptable (success=true, transient_transport=false, tool_calls=0, iterations=1, error=None); giving up
panicked: Should have called get_current_time (tool_calls_count=0, iterations=1)
```

It passed on the next run. So it is flaky rather than a regression — but note
what the harness could actually tell us at the moment of failure: **nothing**.
A clean turn that produced no tool call is indistinguishable from a turn where
we never sent a usable tool definition. Both surface as
`success=true, error=None, tool_calls_count=0`.

That ambiguity is the actual defect. It has two costs, and the second is worse
than the first:

1. Sampling outcomes fail the merge gate with the same severity as a contract
   break, so main goes red for reasons no PR could have caught.
2. A genuine regression in tool serialization, `tool_choice`, or streaming
   parse would present **identically**, and would be dismissed as "the flaky
   OpenRouter test" — which is exactly what happened during triage of
   `834a3a923`.

## What we must not do

**Take the suite off the merge gate.** Attempted in #3141 and closed. Everruns
is an agentic harness engine: provider integration is the product, not an
incidental dependency. The unit-test coverage of the matrix is mocked, so it
encodes our *assumption* of the wire format and passes precisely when that
assumption is wrong. The live matrix is the only thing that catches provider
contract drift, and `main` is what `publish-crates.yml` releases from. A daily
signal also stops attributing a break to the commit that caused it — at the
current merge rate a regression would land under a day of unrelated changes.

**Blanket `continue-on-error`.** Same defect, less honest: it suppresses real
contract breaks along with sampling noise.

**More retries.** The observed failure was three identical clean results, not a
transient fault. Retrying a model that has decided not to call a tool costs
money and time and changes nothing. The existing retry already handles the case
it is good for — `is_transient_transport_error`.

## The shape of the fix

The suite already draws exactly the line we need, for billing.
`is_quota_exhausted` / `skip_if_quota!` treat an out-of-credit account as a live
condition rather than a code regression, and the test returns early. Model
refusal deserves the same treatment — but only once we have independently
established that *our* side of the exchange was correct.

So: two layers, in order.

### Layer 1 — deterministic tool-offer precondition (fails the gate)

Before interpreting a zero-tool-call turn as model behaviour, assert the thing
that is entirely within our control: **we actually offered the tool, and the
provider accepted the offer.**

This is a local, deterministic check. It does not depend on sampling, so it can
run on every attempt of every case and fail hard:

- the turn's request carried a tool definition for the expected tool name;
- its schema serialized to the provider's expected shape;
- the provider did not reject the request (no 4xx/schema error).

If any of these fail, that is a regression in our harness and must redden the
gate — including, importantly, when `tool_calls_count == 0`. This is the layer
that would have caught a real serialization break during the `834a3a923`
triage, instead of it being waved through as a flake.

`TurnResult` as it stands (`response`, `iterations`, `tool_calls_count`,
`success`, `error`, `stop_reason`, `turn_id`) cannot express this. The
precondition needs one of:

- **(a)** a test-only observation of the assembled request — the tool names and
  schemas the harness sent — exposed through the existing event stream, or
- **(b)** a `tools_offered: Vec<String>` (or equivalent count) on `TurnResult`,
  populated by the runtime from the resolved tool set.

(a) is preferable: it uses the event stream we already emit, keeps `TurnResult`
a result rather than a transcript, and gives the assertion real evidence rather
than a restatement of our own intent. (b) is a smaller change if the events do
not carry enough detail. This choice is the main open question below.

### Layer 2 — model-refusal skip (does not fail the gate)

Given Layer 1 held, and the turn was clean — `success == true`,
`error == None`, not a transient transport error — a `tool_calls_count == 0`
after the configured retries is a provider-behaviour outcome. Report it the way
quota is reported:

```
SKIP: <provider>/<model> declined to call <tool> after 3 attempts (turn clean, tool offered)
```

Mirroring the existing macro:

```rust
skip_if_model_declined!(result, config.label(), "get_current_time");
```

The same treatment applies to `test_extended_thinking`, where the absent
artifact is a thinking block rather than a tool call — the case that failed on
`69b1274d5`.

The skip must be narrow. It applies only when every one of these holds:

- Layer 1 passed (we demonstrably offered a well-formed tool);
- `success == true` and `error == None`;
- the retry budget was exhausted;
- the *only* unmet expectation is the model's choice.

Anything else — malformed schema, rejected parameters, transport or contract
errors, a turn that failed outright — still fails.

## Guarding against silent erosion

A skip that nobody reads is a deleted test. If a provider stops calling tools
permanently, or a regression slips past Layer 1, the suite would skip forever
and report green.

Two cheap mitigations, in increasing strength:

1. **Emit a machine-readable skip ledger.** Each skip writes a line the job
   summary aggregates: case, provider, model, reason, date. A human reviewing
   the run sees "3 cases skipped" rather than an unqualified green.
2. **Fail on persistence.** The same case skipping on N consecutive runs (N=3
   is a reasonable start) is no longer noise — it is either a provider that
   changed behaviour or a regression Layer 1 missed. Persistence turns the skip
   back into a failure. This needs somewhere to keep the streak: the simplest
   durable option is a small committed JSON file updated by the workflow, but a
   cache or artifact lookback would avoid the commit churn.

(1) should ship with the change. (2) is worth doing but can follow, provided
the ledger from (1) exists so the streak is reconstructable.

## Why this is better than what it replaces

- A sampling outcome no longer blocks a merge.
- A contract break still does, and now fails *deterministically* on Layer 1
  instead of probabilistically on the model's cooperation — so it is caught on
  the first run rather than whenever sampling happens to expose it.
- Triage stops being guesswork: the failure message states which layer failed,
  so "flaky test" is no longer available as a reflex explanation for a real
  break.
- The gate stays where it belongs, on the merge path.

## Open questions

1. **Layer 1 observability — (a) events or (b) `TurnResult` field?** Needs a
   look at whether the emitted events already carry the resolved tool set with
   enough fidelity to assert schema shape, or only tool names. If only names,
   that still catches "we sent no tool at all" but not "we sent a malformed
   schema"; the provider's own rejection partly covers the latter.
2. **Does a provider reliably reject a malformed tool schema?** If some accept
   it and simply never call the tool, Layer 1's provider-acceptance signal is
   weaker for those providers and the schema assertion has to carry more
   weight.
3. **Retry budget.** With refusal reclassified as a skip, is 3 still right?
   Fewer attempts save money on a case that will be skipped anyway; more
   attempts reduce skip frequency. Worth setting from observed skip rates once
   the ledger exists.
4. **Scope.** This proposal covers `agent_run_basic` and
   `agent_run_with_thinking`. `subagent_live_test` should be audited for the
   same ambiguity — it has its own quota-skip handling already
   (`fix-subagent-live-quota-skip` suggests prior trouble in this area).

## Files this touches

- `crates/llm-tests/tests/llm_test_matrix/mod.rs` — `skip_if_model_declined!`
  beside `skip_if_quota!`; Layer 1 assertion helper; skip ledger emission.
- `crates/llm-tests/tests/agent_run_basic.rs` — tool-call assertion sites.
- `crates/llm-tests/tests/agent_run_with_thinking.rs` — thinking-block sites.
- `crates/host/src/runtime.rs` — only under option (b), for the extra
  `TurnResult` field.
- `.github/workflows/ci.yml` — job-summary aggregation of the skip ledger.
