---
title: Platform Chat Agent
description: The managed Agent for Everruns platform conversations, running on Bashkit Worker.
---

**Platform Chat** is a managed Agent that helps you operate your Everruns organization.
It runs on [Bashkit Worker](/built-ins/harnesses/bashkit-worker/), which supplies a sealed,
recoverable Bashkit virtual workspace. The Agent supplies its identity, instructions, platform
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
inherited from [Bashkit Worker](/built-ins/harnesses/bashkit-worker/).

| Capability | Purpose |
|---|---|
| Platform | Platform operations through the session shell |
| Current Time | Ground relative time questions |
| Task Management | Track work within a conversation |
| Prompt Caching | Cache stable prompt content |
| Tool Call Repair | Repair malformed tool calls |
| Human Intent | Preserve consequential user intent across tool calls |
| Web Fetch | Read public web resources and download files |
| Storage | Keep durable session values and secrets |
| Schedules | Arrange follow-up work for the session |
| BTW | Handle lightweight side questions without losing the main thread |
| Message Metadata | Attach structured metadata to messages |
| Retrieval Citations | Retrieve sources for grounded answers |
| Citation Verification | Verify citation support before answering |
| Ask User | Pause for structured user input when required |
| Error Disclosure | Return detailed platform errors to the managed operator Agent |

## Existing conversations

The dedicated Platform Chat harness and its interim Generic binding are retired. Existing platform
conversations move to the managed Agent on Bashkit Worker while keeping their IDs, history, workspace files,
owners, and memory. Older custom agents, apps, triggers, and child harnesses retain
their original execution bindings so their authored behavior is preserved; the retired
harness remains stored for those bindings and historical accounting.

The Agent name `platform-chat` is reserved for the managed assistant. An existing
custom Agent with that name is renamed to `platform-chat-custom-<ID suffix>`; its
ID and bindings stay intact. Clients using its old name should switch to its ID.

Conversation introductions and starters now belong exclusively to Agents. Existing
harness presentation is copied to its assigned Agents when those Agent fields are
empty. Harness presentation fields are deprecated and no longer accepted on writes.
