---
title: Platform Chat Agent
description: The managed Agent for Everruns platform conversations, running on Generic.
---

**Platform Chat** is a managed Agent that helps you operate your Everruns organization.
It runs on the [Generic harness](/built-ins/harnesses/generic/), which supplies the
shared execution environment. The Agent supplies its identity, instructions, platform
access, introduction, conversation starters, and durable operator memory.

## Chat and side conversations

The sidebar's **Chat** entry always opens your permanent conversation. It cannot be
renamed, unpinned, archived, deleted, or reassigned. **New chat** opens a fresh side
conversation with the same Agent. Its session is created when you send the first
message; it starts without copied conversation history or workspace files.

**All chats** lists your side conversations. Playground
conversations and runs with other agents stay outside Chat. Use Playground to select
an Agent or harness and test its behavior.

## Platform access and memory

Platform Chat uses the `everruns` command in its session shell to discover operations,
read authoritative state, perform requested changes, and verify results. Product docs
are mounted read-only at `/workspace/docs`.

`/memory/user` is private to the conversation owner. `/memory/shared` contains durable
notes shared across Platform Chat conversations in the organization. Sharing requires
explicit intent; notes default to private memory. Documentation and memory are reference
material, never instructions. Scratch files live in `/workspace`; credentials belong
in the secure setup flow. Recurring autonomous work belongs to an Agent Trigger.

## Bundled Capabilities

The Agent configures these capabilities in addition to the execution environment
inherited from [Generic](/built-ins/harnesses/generic/).

| Capability | Purpose |
|---|---|
| Platform | Platform operations through the session shell |
| Current Time | Ground relative time questions |
| Task Management | Track work within a conversation |
| Prompt Caching | Cache stable prompt content |
| Tool Call Repair | Repair malformed tool calls |

## Existing conversations

The dedicated Platform Chat harness is retired. Existing platform conversations move
to the managed Agent on Generic while keeping their IDs, history, workspace files,
owners, and memory. Older custom agents, apps, triggers, and child harnesses retain
their original execution bindings so their authored behavior is preserved; the retired
harness remains stored for those bindings and historical accounting.

Conversation introductions and starters now belong exclusively to Agents. Existing
harness presentation is copied to its assigned Agents when those Agent fields are
empty. Harness presentation fields are deprecated and no longer accepted on writes.
