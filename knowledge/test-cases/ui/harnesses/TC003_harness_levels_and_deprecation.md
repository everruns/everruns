---
type: Test Case
title: "TC003: Harness Levels and Deprecation"
description: "Verify minimal harness selection, preserved deprecated selections, incompatible execution-policy rejection, and role restrictions."
tags:
  - everruns
  - test-case
  - ui
  - harnesses
---
# TC003: Harness Levels and Deprecation

## Description

Verify minimal harness selection, preserved deprecated selections, incompatible execution-policy rejection, and role restrictions.

## Preconditions

- Canonical stack with current harnesses and real chat model configured.
- An owner account, a member fixture, and a legacy agent already selecting Generic.
- Disposable agents and supported execution policies; no production resources.

## Test Data

| Variant | Expected |
| --- | --- |
| Initial harness list | Base, Conversation, Worker, Bashkit Worker, Sandbox Worker |
| Show deprecated | Generic and Worker Base included and marked deprecated |
| Legacy Generic agent | Saved selection remains readable |
| Incompatible execution policy | Rejected; prior state retained |

## Steps

1. Open Harnesses in light and dark mode; inspect the current hierarchy and Show deprecated.
2. Create an agent and inspect its harness selector before and after Show deprecated.
3. Edit the legacy Generic agent; verify its existing value is visible without enabling the toggle.
4. Open the Dad Jokes example, inspect Conversation before import, then import into a disposable agent.
5. Inspect effective preview and a real conversation. Conversation exposes time/questions without shell, file, storage, or subagent tools; Worker exposes files/subagents/tasks without shell; Bashkit Worker adds Bashkit.
6. Try saving a Bashkit Worker agent with a full-provider sandbox policy and a Sandbox Worker agent with a Bashkit-only policy. Re-read each agent after rejection.
7. As the member, try assigning a current High-risk capability. Re-read the fixture using the owner.

## Expected Result

- Current presets are discoverable; deprecated presets remain intentionally available, with legacy selected values preserved.
- Effective previews match runtime tools for the selected execution environment.
- Incompatible execution policies and unauthorized high-risk assignment are rejected without partial changes.
- Deprecation alone does not make a saved legacy agent unusable.
