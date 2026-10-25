---
type: Test Case
title: "TC006: Matched Platform Surfaces and Mutation Evidence"
description: "Compare CLI, eager native and bounded native against the same platform fixture while checking invocation metadata, unchanged state and measured context."
tags:
  - everruns
  - test-case
  - platform-chat
  - evaluation
---
# TC006: Matched Platform Surfaces and Mutation Evidence

## Description

Verify the offline comparison's invocation and state evidence before interpreting
its pass rate or token measurements. This is a synthetic interface qualification;
production authorization, transactions and persistence require the hosted cases.
Production Platform Chat remains CLI-only.

## Preconditions

- Build the Rust study and run its tests using the commands in the
  [study README](../../../../evals/platform-capability/README.md).
- Supply an authorized model key through the configured credential mechanism.
- Retain each raw JSONL report separately. Do not replace historical evidence.

## Test Data

| Field | Value |
| --- | --- |
| Surfaces | `cli`, `native-eager`, `native-bounded` |
| Model | Muse Spark 1.3 Contributor |
| Matched corpus | `surface-comparison` |
| Large-result diagnostics | `surface-scale`, deterministic 200-agent fixture |
| Update target | `agent_01triage`, description `Support intake owner` |

## Steps

1. Run the matched corpus on each surface with identical model, repetitions,
   provider and budgets; rotate surface order between repetitions. Save source
   and binary hashes plus each command and environment selector.
2. Inspect the description-update trace. Locate the successful manager-context
   read and the later completed update for the exact target. Verify a nonempty
   reason and the revision read are separate audit metadata. Compare the complete
   initial and final fixture families: only the requested description may change.
3. Inspect confirmed creation: no mutation attempt may precede the confirmation
   turn; existing rows remain identical and only the requested new row appears.
4. Run the fixture regressions for missing reasons, invalid reasons, stale
   revisions and unavailable bounded operations. Verify each rejected operation
   has an explicit rejected status and the affected entity remains unchanged.
   A missing revision with existing context produces a warning, consistent with
   production's documented contract; the stricter task scorer still requires
   acknowledgement for its requested workflow.
5. Inspect bounded requests. The first request contains search only. Search
   results and active optional schemas obey their recorded byte/count bounds;
   a function cannot execute until a later request actually advertises it.
   Disclosed schemas must equal the eager invocation schemas.
6. Run the two large-result diagnostics separately. Check the aggregation returns
   100 agents per model and the targeted lookup returns Scale Agent 0200 with
   `model_01muse`. Inspect result bytes and provider input tokens; retain failed
   attempts rather than reporting only passing trials.
7. Run the existing Framework catalog-lifecycle tests, including plugin addition,
   schema redefinition, removal and bounded disclosure. Inspect the offline HTTP
   prefix regression: historical messages stay identical, while Chat Completions
   additions change its top-level tools array. Do not infer inline-provider cache
   guarantees from this fallback transport.
8. Recompute summaries from the raw reports with `summarize_surfaces`. Verify
   uncached input plus cache reads equals total input, reasoning remains a subset
   of output, and missing billed-cost coverage is explicit.

## Expected Result

All matched tasks pass with exact state preservation and valid invocation metadata.
The bounded arm starts without the eager inventory and exercises only discovered
operations. Raw attempts, infrastructure failures, provider cache observations
and task scores remain distinguishable. Large-result diagnostics are reported
separately from the original eight-task comparison.

This case is acceptance coverage, not a claim of execution. Recorded runs and
their limits belong in the [study results](../../../../evals/platform-capability/README.md).
