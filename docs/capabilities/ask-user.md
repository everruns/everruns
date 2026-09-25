---
title: Ask User
description: Let an agent ask the user a small batch of structured questions, and wait for the answer, instead of guessing or ending the turn in prose.
---

| | |
|---|---|
| **ID** | `ask_user` |
| **Category** | Core |
| **Features** | None |
| **Dependencies** | None |

Gives the agent one tool for collecting decisions it cannot make on its own. Instead of guessing, or ending the turn with a paragraph of questions and hoping the user answers all of them, it asks 1–4 structured questions and waits.

The user sees a card with the questions. Choice questions show described options, and text questions show a free-form field. Answering resumes the turn.

Enabled by default on the [Generic](/built-ins/harnesses/generic/) and [Platform Chat](/built-ins/harnesses/platform-chat/) harnesses. Not on [Base](/built-ins/harnesses/base/), which has no interactive surface.

## When to enable it

Enable it for agents that work *with* a person: anything where the agent's first guess about intent, scope, or preference is likely to be wrong and expensive to undo.

Leave it off for agents that run unattended on a schedule or a trigger. It will not hang them — a client that cannot render a choice gets the model's declared defaults immediately, while text and secret questions are declined — but an agent nobody is watching should be built to decide, not to ask.

## Not a consent gate

This is the boundary worth being clear about before enabling both:

| | `ask_user` | `request_approval` |
|---|---|---|
| For | Decisions and preferences | Permission to act |
| Example | "Which environment?" | "May I delete this bucket?" |
| No answer | Choice defaults; text and secret decline | Stays unresolved |

Choice questions **auto-resolve**. A choice nobody answers falls back to the option the model marked as recommended. Text and secret questions are skipped instead because they have no default value. None of these outcomes grants permission, so a destructive, irreversible, or outward-facing action must go through `request_approval` (the `soft_approval` capability), whose wait does not auto-resolve.

Both are enabled together on the interactive harnesses for exactly this reason: the agent needs somewhere to put a preference so it stops putting permission questions there. The system prompt states the rule, but the capability pairing is what makes it followable.

## Question kinds

**Choice** — 2 to 6 options, single- or multi-select, optionally with a free-text "Something else" path. Options are ordered most-applicable-first, because that is the order a fallback follows.

**Text** — one free-form answer with no options. Use it for open questions such as a branch name. An unanswered text question is declined rather than submitted as an empty answer.

**Secret** — collects a credential. The value is stored encrypted in [session storage](/capabilities/session-storage/) and the agent receives a reference (`session:MY_TOKEN`), never the value itself, so it cannot reach the conversation history or the agent's context. Tools resolve the reference by name. A secret question never auto-resolves: there is no such thing as a default credential, so an unanswered one is declined and proceeding without it becomes the agent's explicit decision.

Use the secret kind rather than asking for a key in chat. A key typed into an ordinary message stays in the session history in plain text.

## Tools

| Tool | Description |
|---|---|
| `ask_user` | Ask 1–4 choice or free-form questions, or collect one credential, and wait for the answer. |

## Configuration

None. Limits are fixed by the contract:

| Limit | Value |
|---|---|
| Questions per call | 1–4 (a secret question must be alone) |
| Options per choice question | 2–6 |
| Timeout | 300 seconds, which the agent may shorten but not extend |

## What the agent is told

The result says what was chosen **and who chose it** — a person, a timeout, or an unattended fallback. An agent that reads a fallback as a considered answer will act with more confidence than the answer deserves, so provenance travels with it.

A declined question is a finished decision. The agent must not ask it again.

## See Also

- [Session Storage](/capabilities/session-storage/), where a collected secret is kept
- [Implementing a responder](/framework/ask-user/), for embedding applications
- [Capabilities Overview](/capabilities/)
