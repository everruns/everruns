---
title: Soft Approval
description: Prompt-level consent gate. The agent pauses and asks before destructive or outward-facing actions, and runs safe work without interruption.
appliesTo: [framework, platform, cloud]
---

| | |
|---|---|
| **ID** | `soft_approval` |
| **Category** | Safety |
| **Tools** | `request_approval`, `record_approval`, `set_approval_mode` |
| **Features** | None |
| **Dependencies** | None |

Soft approval asks the agent to stop and get the user's consent before a
critical action, and to run everything else without asking. It works through
the system prompt and three tools. It is guidance for the model, not an
enforcement point: a call the model makes still runs.

When a call must not run without a person's decision, use
[Tool Approval](/capabilities/tool-approval/) instead. That gate holds the call
until someone approves it, whatever the model does.

Worker Base, Worker, deprecated [Generic](/built-ins/harnesses/generic/) and
the [Platform Chat](/built-ins/harnesses/platform-chat/) include soft
approval at the `normal` level.

## Levels

| Mode | The agent asks before |
|---|---|
| `off` | Nothing. The capability adds no prompt. |
| `normal` (default) | Destructive or irreversible actions (deleting files or data, dropping records, force-push, history rewrites) and outward-facing ones (pushing, publishing, opening pull requests, sending messages or mail, deploying, spending money). Ordinary edits and local, reversible changes proceed. |
| `protective` | Any action that changes state: writing or deleting files, commits and pushes, installing or removing packages, requests with side effects, changes to shared resources, and any command that is not plainly read-only. |

At every level except `off`, the prompt tells the agent to plan, batch the safe
steps, and never pause on read-only inspection.

## Configuration

```json
{
  "ref": "soft_approval",
  "config": { "mode": "normal" }
}
```

`mode` accepts `off`, `normal`, or `protective`, and the synonyms listed under
[`set_approval_mode`](#set_approval_mode). Any other value fails validation.

## Tools

### `request_approval`

The agent calls it at a critical action, then ends its turn. The call is the
pause: it tells the user the agent is waiting.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `action` | string | Yes | The action awaiting approval, for example "delete the staging database" |
| `question` | string | No | One short question for the user. Defaults to "Go ahead?" |

The result carries `awaiting_approval: true`, the action and question, and the
turn and input message it was asked in.

### `record_approval`

After the user says yes, the agent calls it with what was approved, then
carries the action out. A plain affirmative counts as approval; a hesitant or
negative reply does not.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `action` | string | Yes | The specific action the user approved |
| `detail` | string | No | Extra context such as the command or scope. It is logged, so it must not contain secrets. |

The result records the turn and input message the consent was given in. An
auditor can join that message to the host's own record of who sent it.

One approval covers one action. A category the user pre-authorizes ("no need to
ask for commits"), or a workflow the user invoked by name, covers the actions it
names; the agent records that grant once.

### `set_approval_mode`

When the user asks the agent to be more or less careful, the agent calls it
with `mode`. It accepts `protective`, `normal`, and `off`, plus synonyms such
as `paranoid`, `careful`, `default`, and `yolo`. The new level overrides the
configured `mode` for that session from the next turn on.

## Relation to Ask User

[Ask User](/capabilities/ask-user/#not-a-consent-gate) questions auto-resolve,
so an answer to them never grants permission. Soft approval's wait does not
auto-resolve. The interactive harnesses enable both, so the agent has a place
for preference questions and a separate place for consent.

## See also

- [Tool Approval](/capabilities/tool-approval/), the hard approval gate
- [Ask User](/capabilities/ask-user/), structured questions to the user
- [Guardrails](/capabilities/guardrails/), config-driven checks over output and tool activity
