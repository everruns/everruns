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

## Chat and subchats

The sidebar's **Chat** entry always opens your permanent conversation. It cannot be
renamed, unpinned, archived, deleted, or reassigned. **New subchat** opens a fresh
conversation with the same Agent. Its session is created when you send the first
message; it starts without copied conversation history or workspace files.

Each subchat is a new Chat instance with its own transcript and workspace files.
Selecting a subchat replaces the main conversation; Chat never opens a second
conversation beside itself. The Subchats panel lists your conversations.

**All chats** lists your subchats. Playground
conversations and runs with other agents stay outside Chat. Use Playground to select
an Agent or harness and test its behavior.

## Chat beside your pages

Organization owners and admins can enable **Chat workspace** in **Settings > Features**.
Open an app page, then press **Command-J** (**Ctrl-J** on Windows and Linux).
Your selected personal Chat opens beside the page, defaulting to the permanent
conversation. **Back to app** restores the full page and keeps the conversation
and composer draft. The sidebar's **Chat** entry opens full Chat; on Chat pages,
Command/Ctrl-J focuses the existing composer. Page context is attached only
when Chat is beside an app page.

Each message shares the current page's identity with Chat. On an Agent page,
Chat can read the live form, including unsaved instructions and configuration,
and apply requested edits to that draft. Gold markers identify **Draft edited by
Chat**. Review the result, then use **Save changes** to persist it or **Discard**
to reset the draft. Draft **Undo** restores the preceding form values, including
your earlier manual edits, and refuses to overwrite later manual edits. If you
change the form while Chat is thinking, its stale patch fails so it can read the
current draft again. Chat does not edit credentials or connection grants through
this form; sandbox policy uses its dedicated editor.

For separately requested saved-resource operations, Platform Chat still uses
platform commands. Page context does not disable those operations, and a failed
form edit is never permission to bypass review by saving directly. Supported resource saves attributed to the conversation receive saved-change
review and **Undo all**. That Undo creates a history entry and checks the saved
revision; unsupported inverses fail without saving. If a saved definition changes
while your draft is dirty, the page keeps the draft and blocks Save until you
discard it and review the latest definition.

Chat can read page identity, current filters and bounded summaries of loaded records
across the app. It reads explicit data, without copying page HTML or credential
inputs. Harnesses, Virtual Users, Observers, Eval creation and other definition
forms expose supported draft fields through the same read/edit flow. Knowledge,
Memory, Skill, Sandbox-template, Capability and Channel forms expose their supported
non-credential fields. Providers with several Save forms identify each form by name;
Chat edits one form at a time.

Use the page's existing **Save** or **Create** action to apply a staged change.
Organization, Model defaults and Web search defaults use explicit **Save** and
**Discard** controls while Chat workspace is enabled. Chat can also stage supported
settings, schedule and eval-case fields, MCP metadata, invitations, report-builder
choices and Playground setup. Connection credentials, approvals, financial actions
and operational buttons retain their normal manual controls. Operational context
includes bounded status rows and the current Monitoring view. Chat reports when a
page or field is read-only; it does not work around that through a server write.
Supported existing-resource editors share saved **Before/After** review,
field-level **Undo**, **Undo all** and **History**. Save checks the version you
started editing; Undo refuses to overwrite newer changes. Reviews follow you
between pages in the same organization. Model selections save and undo as one
batch. Account and Sandbox Template metadata inverses preserve current credentials
and execution settings. Profile history is private to its owner.

New-resource forms still require Create. Undo does not delete newly created
resources or reverse invitations, payments, issued tokens or queued tasks.
Protected values keep their manual controls. An inverse that no longer passes
validation stays visible with an error for review.
Chat workspace is independent of **Chat background work**, which enables
delegation and the Work tab. Each Chat instance has its own work. Core Chat,
subchats and history remain available with either Adoption feature off.

## Platform access and memory

Platform Chat uses native `platform_<wire_name>` functions for individual operations,
with bounded `tool_search` to find and reveal schemas as needed. Its `everruns` command
and session shell remain available for loops, filtering, and aggregation across many
operations. Product docs are mounted read-only at `/workspace/docs`. The Platform capability also supplies
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
| Coordination | Hand focused work to threads you can open and follow, when the organization has adopted Chat background work |
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
