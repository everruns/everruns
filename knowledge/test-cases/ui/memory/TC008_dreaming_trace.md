---
type: Test Case
title: "TC008: Memory - Background dreaming and Trace"
description: Verify background consolidation produces authorized Memory notes and a separate metadata-only Trace lane.
tags: [everruns, memory, dreaming, trace]
---
# TC008: Memory - Background dreaming and Trace

## Description

Verify background consolidation produces authorized Memory notes and a separate metadata-only Trace lane.

## Preconditions

Canonical local stack with `AUTH_MODE=none`, a real priced model, organization Adoption flag `dreaming` enabled, and an agent with writable user Memory and Dreaming configured. Use an isolated session and record its ID and initial event cursor. See [Dreaming](../../../runtime-resources/dreaming.md).

## Test Data

| Input | Value |
| --- | --- |
| Preference | Prefer concise Rust examples and UTC timestamps |
| Trigger | Manual |
| Destination | `/memory/user/learned` |

## Steps

1. Send the preference and wait for turn completion. Run `/dream`.
2. Within 120 seconds, confirm new `dreaming.activity` events report completion and a source-linked artifact exists in the configured Memory directory.
3. Open Trace and expand Background dreaming. Confirm status, note count, model, elapsed time, token usage and cost are visible without note text or evidence.
4. Create a new session sharing that Memory; ask for saved preferences and verify it reads the artifact.
5. Repeat `/dream` without new experience. Re-read the Memory files and confirm no duplicate artifact.
6. Repeat with proposal mode in a fresh session. Confirm active Memory is unchanged before `/dream accept`; inspect `/dream proposals`, accept, and verify the artifact. In another proposal session, discard and verify no artifact.

7. Disable the organization Dreaming feature flag. Confirm `/dream` is unavailable and new Worker sessions do not run dreaming. Re-enable it and confirm Worker inheritance becomes available without adding the capability to the agent.

## Expected Result

Foreground conversation remains intact. Background duration is separate from foreground duration. Memory persistence and cross-session recall succeed; retry and review do not duplicate or overwrite files. Trace contains metadata only. A missing or read-only Memory destination fails without creating an artifact.
