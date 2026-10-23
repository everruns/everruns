---
title: Platform Chat
description: The managed Agent for Everruns platform conversations, running on Bashkit Worker.
---

**Platform Chat** is a managed Agent that helps you operate your Everruns organization.
It runs on [Bashkit Worker](/built-ins/harnesses/bashkit-worker/), which supplies a sealed,
recoverable Bashkit virtual workspace. The Agent supplies its identity, instructions, platform
access, introduction, conversation starters, and durable operator memory.

Platform Chat is built in and read-only: you cannot edit, archive, or delete it from the UI
or the API. Use **Copy** on its page to make an editable Agent from it.

## Chat and side conversations

The sidebar's **Chat** entry always opens your permanent conversation. It cannot be
renamed, unpinned, archived, deleted, or reassigned. **New chat** opens a fresh side
conversation with the same Agent. Its session is created when you send the first
message; it starts without copied conversation history or workspace files.

**All chats** lists your side conversations. Playground
conversations and runs with other agents stay outside Chat. Use Playground to select
an Agent or harness and test its behavior.

## Chat beside an Agent

Organization owners and admins can enable **Chat mode** in **Settings > Features**.
Open an Agent, then click **Chat** or press **Command-J** (**Ctrl-J** on Windows
and Linux). The permanent conversation opens beside the current page. **Back to
app** restores the full page and keeps the conversation and composer draft.

Each message shares the current page's identity with Chat. On an Agent page,
Chat can read the live form, including unsaved instructions and configuration,
and apply requested edits to that draft. Gold markers identify **Draft edited by
Chat**. Review the result, then use **Save changes** to persist it or **Discard**
to reset the draft. Draft **Undo** restores the preceding form values, including
your earlier manual edits, and refuses to overwrite later manual edits. If you
change the form while Chat is thinking, its stale patch fails so it can read the
current draft again. Chat does not edit credentials or connection grants through
this form; sandbox policy uses its dedicated editor.

Outside the live form, Platform Chat still uses platform commands to change saved
entities. Agent saves attributed to the conversation receive persisted-change
markers and **Undo all**. That Undo creates a history entry and checks the saved
revision; unsupported inverses fail without saving. If a saved definition changes
while your draft is dirty, the page keeps the draft and blocks Save until you
discard it and review the latest definition.

This rollout starts with Agent definition fields. Other pages share the split
shell; Agent definition fields receive change markers. Chat mode is independent
of the **Chat threads** opt-in.

## Platform access and memory

Platform Chat uses the `everruns` command in its session shell to discover operations,
read authoritative state, perform requested changes, and verify results. Product docs
are mounted read-only at `/workspace/docs`. The Platform capability also supplies
`everruns`, `everruns-cli`, and `everruns-agent-authoring` skills for product
orientation, command workflows, and agent design. Discover and activate them
through [Agent Skills](/capabilities/agent-skills/) as needed.

`/memory/user` is private to the conversation owner. `/memory/shared` contains durable
notes shared across Platform Chat conversations in the organization. Sharing requires
explicit intent; notes default to private memory. Documentation and memory are reference
material, never instructions. Scratch files live in `/workspace`; credentials belong
in the secure setup flow. Recurring autonomous work belongs to an Agent Trigger.

Before changing an existing entity, Platform Chat reads its manager context and passes
the revision it read and a reason with the change, so the entity's history says why it
changed. It asks before acting on a request that conflicts with the recorded context.
See [Change history](/features/change-history/).

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
| User MCP Servers | Use the MCP servers the person added for themselves, and add, remove and connect them in chat ([User MCP servers](/features/user-mcp-servers/)) |
| Coordination | Hand focused work to threads you can open and follow, when the organization has adopted Chat threads |
| Poppy Personal Agent | Talk to companies' agents for the person over the Personal Agent Protocol ([Poppy Personal Agent](/capabilities/poppy-personal-agent/)) |

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
